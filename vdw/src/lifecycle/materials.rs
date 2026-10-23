use super::{io::*, types::*};
use anyhow::{bail, Context, Result};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub fn matches(patterns: &[String], path: &str) -> bool {
    patterns.iter().any(|p| {
        glob::Pattern::new(p)
            .map(|g| {
                g.matches(path)
                    || p.strip_suffix("/**")
                        .map(|base| path == base)
                        .unwrap_or(false)
            })
            .unwrap_or(false)
    })
}
pub fn validate_patterns(patterns: &[String]) -> Result<()> {
    for p in patterns {
        glob::Pattern::new(p)?;
    }
    Ok(())
}

pub fn collect(
    root: &Path,
    dependencies: &[String],
    cfg: &MaterialConfig,
) -> Result<Vec<Material>> {
    // Normalize declarations before comparing them with collected project paths.
    let dependencies = dependencies
        .iter()
        .map(|path| Ok(safe_relative(path)?.to_string_lossy().replace('\\', "/")))
        .collect::<Result<Vec<_>>>()?;
    let mut result = vec![];
    fn visit(
        root: &Path,
        path: &Path,
        deps: &[String],
        cfg: &MaterialConfig,
        result: &mut Vec<Material>,
    ) -> Result<()> {
        let rel = path
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        let is_runtime = rel == ".git"
            || rel.split('/').any(|p| p == ".git")
            || [
                ".vdw/state.json",
                ".vdw/identity.json",
                ".vdw/archive.json",
                ".vdw/runtime",
            ]
            .iter()
            .any(|p| rel == *p || rel.starts_with(&format!("{p}/")));
        if is_runtime
            || deps
                .iter()
                .any(|d| rel == *d || rel.starts_with(&format!("{d}/")))
            || matches(&cfg.exclude, &rel)
        {
            return Ok(());
        }
        let meta = fs::symlink_metadata(path)?;
        #[cfg(unix)]
        let executable = {
            use std::os::unix::fs::PermissionsExt;
            meta.permissions().mode() & 0o111 != 0
        };
        #[cfg(not(unix))]
        let executable = false;
        if meta.file_type().is_symlink() {
            let target = fs::read_link(path)?.to_string_lossy().into_owned();
            if matches(&cfg.include, &rel) {
                result.push(Material {
                    path: rel,
                    kind: "symlink".into(),
                    bytes: 0,
                    hash: hash(target.as_bytes()),
                    link_target: Some(target),
                    executable: false,
                });
            }
        } else if meta.is_dir() {
            if path.join(".git").exists() {
                bail!("Unmanaged nested Git repository: {}", path.display());
            }
            // Directories are recorded too, so empty project directories survive.
            result.push(Material {
                path: rel,
                kind: "directory".into(),
                bytes: 0,
                hash: hash(b"directory"),
                link_target: None,
                executable: false,
            });
            let mut children = fs::read_dir(path)?
                .map(|e| e.map(|e| e.path()))
                .collect::<std::io::Result<Vec<_>>>()?;
            children.sort();
            for child in children {
                visit(root, &child, deps, cfg, result)?;
            }
        } else if meta.is_file() {
            if matches(&cfg.include, &rel) {
                result.push(Material {
                    path: rel,
                    kind: "file".into(),
                    bytes: meta.len(),
                    hash: hash_file(path)?,
                    link_target: None,
                    executable,
                });
            }
        } else {
            bail!("Unsupported project file: {}", path.display());
        }
        Ok(())
    }
    let mut children = fs::read_dir(root)?
        .map(|e| e.map(|e| e.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    children.sort();
    for child in children {
        visit(root, &child, &dependencies, cfg, &mut result)?;
    }
    Ok(result)
}

pub fn copy_materials(
    source: &Path,
    dest: &Path,
    entries: &[Material],
    allow_existing: bool,
) -> Result<()> {
    private_dir(dest)?;
    for entry in entries {
        let rel = safe_relative(&entry.path)?;
        let parent = rel.parent().unwrap_or(Path::new(""));
        let target_parent = if parent.as_os_str().is_empty() {
            dest.to_path_buf()
        } else {
            confined(dest, &parent.to_string_lossy())?
        };
        let target = target_parent.join(rel.file_name().context("Missing file name")?);
        if fs::symlink_metadata(&target).is_ok() {
            if !allow_existing {
                bail!("Archive target already exists: {}", target.display());
            }
            if entry.kind == "directory" {
                if !target.is_dir() || fs::symlink_metadata(&target)?.file_type().is_symlink() {
                    bail!("Directory conflict: {}", entry.path);
                }
                continue;
            }
            if entry.kind == "symlink"
                && fs::symlink_metadata(&target)?.file_type().is_symlink()
                && fs::read_link(&target)?.to_string_lossy()
                    == entry.link_target.as_deref().unwrap_or("")
            {
                continue;
            }
            if entry.kind == "file"
                && !fs::symlink_metadata(&target)?.file_type().is_symlink()
                && hash_file(&target)? == entry.hash
            {
                continue;
            }
            bail!("Project material conflict; preserve current file or choose a new restore directory: {}",entry.path);
        }
        private_dir(&target_parent)?;
        match entry.kind.as_str() {
            "directory" => private_dir(&target)?,
            "file" => {
                let src = confined(source, &entry.path)?;
                if hash_file(&src)? != entry.hash {
                    bail!("Project file changed: {}", entry.path);
                }
                let mut input = fs::File::open(src)?;
                let mut opts = fs::OpenOptions::new();
                opts.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    opts.mode(if entry.executable { 0o700 } else { 0o600 });
                }
                let mut output = opts.open(&target)?;
                std::io::copy(&mut input, &mut output)?;
                output.sync_all()?;
                if hash_file(&target)? != entry.hash {
                    bail!("Copied file checksum mismatch: {}", entry.path);
                }
            }
            "symlink" => {
                let recorded = entry
                    .link_target
                    .as_deref()
                    .context("Missing symlink target")?;
                // Copy link metadata without reading its referent.
                if fs::read_link(source.join(&rel))?.to_string_lossy() != recorded {
                    bail!("Symlink changed: {}", entry.path);
                }
                #[cfg(unix)]
                std::os::unix::fs::symlink(recorded, &target)?;
                #[cfg(not(unix))]
                bail!("Symlink material restoration requires an explicitly supported platform");
            }
            _ => bail!("Unknown material kind"),
        }
    }
    Ok(())
}

pub fn preflight_restore(dest: &Path, entries: &[Material]) -> Result<()> {
    for e in entries {
        let rel = safe_relative(&e.path)?;
        let parent = rel.parent().unwrap_or(Path::new(""));
        let p = if parent.as_os_str().is_empty() {
            dest.to_path_buf()
        } else {
            confined(dest, &parent.to_string_lossy())?
        }
        .join(rel.file_name().unwrap());
        if let Ok(meta) = fs::symlink_metadata(&p) {
            let same = match e.kind.as_str() {
                "directory" => meta.is_dir() && !meta.file_type().is_symlink(),
                "symlink" => {
                    meta.file_type().is_symlink()
                        && fs::read_link(&p)?.to_string_lossy()
                            == e.link_target.as_deref().unwrap_or("")
                }
                "file" => {
                    meta.is_file() && !meta.file_type().is_symlink() && hash_file(&p)? == e.hash
                }
                _ => false,
            };
            if !same {
                bail!("Project material conflict: {}", e.path);
            }
        }
    }
    Ok(())
}

pub fn index(snapshot: &Path, manifest: &Manifest) -> Result<()> {
    let mut sources = String::new();
    let mut explicit = vec![];
    let mut catalog = format!(
        "# {}\n\nArchive: {}\nReason: {}\n\n## Repositories\n\n",
        manifest.name, manifest.archive_id, manifest.reason
    );
    for r in &manifest.repositories {
        catalog.push_str(&format!(
            "- {}: branch {:?}, HEAD {}, lock {}\n",
            r.path, r.branch, r.head, r.locked_commit
        ));
    }
    catalog.push_str("\n## Project materials\n\n");
    for e in &manifest.materials {
        sources.push_str(&serde_json::to_string(e)?);
        sources.push('\n');
        catalog.push_str(&format!(
            "- {} ({}, {} bytes, {})\n",
            e.path, e.kind, e.bytes, e.hash
        ));
        if e.kind != "file" || e.bytes > 1024 * 1024 {
            continue;
        }
        if !(e.path.ends_with(".md") || e.path.ends_with(".txt")) {
            continue;
        }
        let text = match fs::read_to_string(confined(&snapshot.join("workspace-files"), &e.path)?) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let mut section = String::new();
        for (n, line) in text.lines().enumerate() {
            if line.starts_with('#') {
                section = line.trim_start_matches('#').trim().to_string();
            }
            let labeled = [
                "决策",
                "问题",
                "改进",
                "待办",
                "decision",
                "todo",
                "improvement",
            ]
            .iter()
            .any(|label| section.to_lowercase().contains(label));
            if (labeled && !line.starts_with('#') && !line.trim().is_empty())
                || line.starts_with("- [")
            {
                explicit.push(serde_json::json!({"source_path":e.path,"source_hash":e.hash,"line":n+1,"section":section,"text":line,"verification_state":"recorded_unverified"}));
            }
        }
    }
    catalog.push_str("\n## Restore\n\nUse `vdw workspace restore <archive_id> --preview`. Repository tests, releases, and business acceptance are unverified unless original evidence explicitly records them.\n");
    atomic_bytes(&snapshot.join("memory/catalog.md"), catalog.as_bytes())?;
    atomic_bytes(&snapshot.join("memory/sources.jsonl"), sources.as_bytes())?;
    save(&snapshot.join("memory/explicit-notes.json"), &explicit)
}

pub fn archive_files(root: &Path) -> Result<BTreeMap<String, String>> {
    let mut files = BTreeMap::new();
    for e in walkdir::WalkDir::new(root).follow_links(false) {
        let e = e?;
        if e.file_type().is_file() {
            files.insert(
                e.path()
                    .strip_prefix(root)?
                    .to_string_lossy()
                    .replace('\\', "/"),
                hash_file(e.path())?,
            );
        }
    }
    Ok(files)
}

pub fn validate_entries(entries: &[Material]) -> Result<()> {
    let mut types = BTreeMap::<PathBuf, String>::new();
    for entry in entries {
        let rel = safe_relative(&entry.path)?;
        for parent in rel.ancestors().skip(1) {
            if types.get(parent).map(|k| k != "directory").unwrap_or(false) {
                bail!("Material parent is not a directory");
            }
        }
        if types.insert(rel, entry.kind.clone()).is_some() {
            bail!("Duplicate material path");
        }
    }
    Ok(())
}
