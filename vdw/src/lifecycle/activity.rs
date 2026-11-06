//! Read source timestamps without treating editor launches or VDW runtime files as edits.
use super::io::{absolute, confined, safe_relative};
use crate::config::parser;
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use std::{collections::BTreeSet, fs, path::Path};

pub fn last_modified_at(root: &Path) -> Result<Option<DateTime<Utc>>> {
    let root = absolute(root)?;
    let declaration = parser::load_config(&root)?;
    let dependencies = declaration
        .dependencies
        .iter()
        .map(|dep| safe_relative(&dep.path))
        .collect::<Result<Vec<_>>>()?;
    let mut latest = None;
    // Workspace notes/configuration are independent of managed repository projections.
    visit(&root, &dependencies, &mut latest)?;
    for dep in declaration.dependencies {
        let path = confined(&root, &dep.path)?;
        if dep.editable {
            let repo = git2::Repository::open(&path)?;
            if repo.workdir().and_then(|p| p.canonicalize().ok()) != Some(path.canonicalize()?) {
                bail!("Dependency is not an independent Git worktree");
            }
            let mut files = BTreeSet::new();
            for entry in repo.index()?.iter() {
                files.insert(std::str::from_utf8(&entry.path)?.to_owned());
            }
            let mut options = git2::StatusOptions::new();
            options
                .include_untracked(true)
                .recurse_untracked_dirs(true)
                .update_index(false);
            for entry in repo.statuses(Some(&mut options))?.iter() {
                if let Some(name) = entry.path() {
                    files.insert(name.to_owned());
                }
            }
            for file in files {
                let rel = safe_relative(&file)?;
                let parent = rel.parent().unwrap_or(Path::new(""));
                let parent = if parent.as_os_str().is_empty() {
                    path.clone()
                } else {
                    confined(&path, parent.to_str().context("Non-UTF8 source path")?)?
                };
                let file = parent.join(rel.file_name().context("Missing source filename")?);
                match fs::symlink_metadata(&file) {
                    Ok(meta) => record(&meta, &mut latest)?,
                    // For deleted tracked files the containing directory records the removal.
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        let mut cursor = parent.as_path();
                        while !cursor.exists() {
                            cursor = cursor.parent().context("Missing source parent")?;
                        }
                        record(&fs::symlink_metadata(cursor)?, &mut latest)?;
                    }
                    Err(e) => return Err(e.into()),
                }
            }
        } else {
            visit(&path, &[], &mut latest)?;
        }
    }
    Ok(latest)
}

fn record(meta: &fs::Metadata, latest: &mut Option<DateTime<Utc>>) -> Result<()> {
    let modified = DateTime::<Utc>::from(meta.modified()?);
    *latest = Some(latest.map(|value| value.max(modified)).unwrap_or(modified));
    Ok(())
}

fn visit(
    root: &Path,
    excluded: &[std::path::PathBuf],
    latest: &mut Option<DateTime<Utc>>,
) -> Result<()> {
    for entry in walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            entry.depth() == 0
                || (!matches!(
                    entry.file_name().to_str(),
                    Some(
                        ".git"
                            | ".vdw"
                            | "node_modules"
                            | "target"
                            | "dist"
                            | "build"
                            | ".cache"
                            | "__pycache__"
                            | ".DS_Store"
                    )
                ) && !excluded.iter().any(|path| {
                    entry
                        .path()
                        .strip_prefix(root)
                        .map(|rel| rel.starts_with(path))
                        .unwrap_or(false)
                }))
        })
    {
        let entry = entry?;
        if entry.file_type().is_file() || entry.file_type().is_symlink() {
            record(&fs::symlink_metadata(entry.path())?, latest)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    #[test]
    fn source_dates_ignore_runtime_outputs_and_do_not_write() {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join(".vdwrc"),
            "workspace:\n  name: dates\n  version: \"1.0\"\ndependencies: []\n",
        )
        .unwrap();
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        fs::File::options()
            .write(true)
            .open(root.path().join(".vdwrc"))
            .unwrap()
            .set_modified(old)
            .unwrap();
        for directory in [".vdw", "node_modules", "target"] {
            fs::create_dir(root.path().join(directory)).unwrap();
            fs::write(
                root.path().join(directory).join("runtime"),
                "newer but ignored",
            )
            .unwrap();
        }
        let first = last_modified_at(root.path()).unwrap();
        assert_eq!(first, Some(DateTime::<Utc>::from(old)));
        assert_eq!(last_modified_at(root.path()).unwrap(), first);
        fs::write(root.path().join("notes.md"), "source edit").unwrap();
        assert!(last_modified_at(root.path()).unwrap() > first);
    }

    #[test]
    fn repository_dates_include_untracked_edits_and_deletions_but_ignore_git_cache() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("packages/sample");
        fs::create_dir_all(&source).unwrap();
        let repo = git2::Repository::init(&source).unwrap();
        fs::write(source.join("tracked.txt"), "tracked").unwrap();
        fs::write(source.join(".gitignore"), "ignored.log\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("tracked.txt")).unwrap();
        index.add_path(Path::new(".gitignore")).unwrap();
        index.write().unwrap();
        fs::write(root.path().join(".vdwrc"), serde_json::to_vec(&serde_json::json!({
            "workspace":{"name":"source-dates","version":"1.0"},"dependencies":[{"local":source,"path":"packages/sample","editable":true,"branch":"main","scope":{"include":["**/*"],"exclude":[]}}]
        })).unwrap()).unwrap();
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        for file in [
            root.path().join(".vdwrc"),
            source.join("tracked.txt"),
            source.join(".gitignore"),
        ] {
            fs::File::options()
                .write(true)
                .open(file)
                .unwrap()
                .set_modified(old)
                .unwrap();
        }
        fs::write(source.join("ignored.log"), "generated").unwrap();
        assert_eq!(
            last_modified_at(root.path()).unwrap(),
            Some(DateTime::<Utc>::from(old))
        );
        let edit = old + Duration::from_secs(60);
        fs::write(source.join("untracked.txt"), "new work").unwrap();
        fs::File::options()
            .write(true)
            .open(source.join("untracked.txt"))
            .unwrap()
            .set_modified(edit)
            .unwrap();
        assert_eq!(
            last_modified_at(root.path()).unwrap(),
            Some(DateTime::<Utc>::from(edit))
        );
        fs::remove_file(source.join("tracked.txt")).unwrap();
        assert!(last_modified_at(root.path()).unwrap() > Some(DateTime::<Utc>::from(edit)));
    }
}
