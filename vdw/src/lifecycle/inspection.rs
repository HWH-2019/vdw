//! Read-only repository checks shared by dashboard status and lifecycle preflight.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fs, io::ErrorKind, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArchiveNotice {
    pub code: String,
    pub path: String,
    pub message: String,
}

#[derive(Debug, Default)]
pub struct SubmoduleInspection {
    pub blockers: Vec<ArchiveNotice>,
    pub warnings: Vec<ArchiveNotice>,
}

fn has_content(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {
            Ok(fs::read_dir(path)?.next().transpose()?.is_some())
        }
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn mapping_paths(root: &Path) -> Result<BTreeSet<String>> {
    let path = super::io::confined(root, ".gitmodules")?;
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(BTreeSet::new()),
        Err(error) => return Err(error.into()),
        Ok(meta) if !meta.is_file() => bail!(".gitmodules 不是可读取的普通文件"),
        Ok(_) => {}
    }
    let config = git2::Config::open(&path).context(".gitmodules 无法解析，不能确认子模块状态")?;
    let mut mappings = BTreeSet::new();
    let mut entries = config.entries(None)?;
    while let Some(entry) = entries.next() {
        let entry = entry?;
        if entry
            .name()
            .is_some_and(|name| name.starts_with("submodule.") && name.ends_with(".path"))
        {
            if let Some(value) = entry.value() {
                mappings.insert(value.trim_start_matches("./").trim_end_matches('/').into());
            }
        }
    }
    Ok(mappings)
}

fn has_submodule_data(repo: &git2::Repository, root: &Path) -> Result<bool> {
    // Deinitialized modules can retain unique commits outside the empty worktree.
    // Without mappings, arbitrary module names cannot be matched to gitlinks.
    let common = super::io::git(
        root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    for admin in [repo.path(), Path::new(&common)] {
        if has_content(&super::io::confined(admin, "modules")?)? {
            return Ok(true);
        }
    }
    let config = repo.config()?;
    let mut entries = config.entries(None)?;
    while let Some(entry) = entries.next() {
        let entry = entry?;
        if entry.name().is_some_and(|name| {
            name.starts_with("submodule.") && (name.ends_with(".url") || name.ends_with(".gitdir"))
        }) {
            // An explicitly configured external Git directory must not be lost.
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn inspect_submodules(path: &Path) -> Result<SubmoduleInspection> {
    let repo = git2::Repository::open(path)?;
    let index = repo.index()?;
    let mut links = std::collections::BTreeMap::new();
    for entry in index.iter().filter(|entry| entry.mode == 0o160000) {
        if entry.flags & 0x3000 != 0 {
            bail!("子模块索引存在未解决冲突，不能确认归档安全");
        }
        links.insert(
            String::from_utf8(entry.path).context("子模块路径无法读取")?,
            entry.id.to_string(),
        );
    }
    if links.is_empty() {
        return Ok(SubmoduleInspection::default());
    }
    let mappings = mapping_paths(path)?;
    let retained_data = has_submodule_data(&repo, path)?;
    let mut inspection = SubmoduleInspection::default();
    for (relative, commit) in links {
        let directory = super::io::confined(path, &relative)?;
        let content = has_content(&directory)
            .with_context(|| format!("无法读取子模块目录 {relative}，不能确认归档安全"))?;
        let mapped = mappings.contains(&relative);
        if !mapped && !content && !retained_data {
            inspection.warnings.push(ArchiveNotice {
                code: "empty_orphan_submodule".into(), path: relative.clone(),
                message: format!("空子模块指针 {relative}：缺少 .gitmodules 映射，本地无内容且无子仓库数据；允许归档并原样保留指针，恢复后仍未初始化。固定提交 {}", &commit[..12]),
            });
        } else {
            let mut facts = Vec::new();
            if content {
                facts.push("本地目录包含文件或子仓库数据");
            }
            if retained_data {
                facts.push("存在留存的子模块 Git 数据或初始化配置");
            }
            if mapped {
                facts.push("存在 .gitmodules 映射，当前尚不支持完整的子模块归档");
            }
            inspection.blockers.push(ArchiveNotice {
                code: "unsupported_submodule".into(),
                path: relative.clone(),
                message: format!(
                    "子模块 {relative} 阻止归档：{}。固定提交 {}",
                    facts.join("；"),
                    &commit[..12]
                ),
            });
        }
    }
    Ok(inspection)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn orphan() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(root.path()).unwrap();
        let signature = git2::Signature::now("Fixture", "fixture@example.invalid").unwrap();
        fs::write(root.path().join(".gitmodules"), "").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new(".gitmodules")).unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let base = repo
            .commit(Some("HEAD"), &signature, &signature, "base", &tree, &[])
            .unwrap();
        assert!(std::process::Command::new("git")
            .arg("-C")
            .arg(root.path())
            .args([
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("160000,{base},nested")
            ])
            .status()
            .unwrap()
            .success());
        fs::create_dir(root.path().join("nested")).unwrap();
        index.read(true).unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            "orphan gitlink",
            &tree,
            &[&repo.find_commit(base).unwrap()],
        )
        .unwrap();
        root
    }

    #[test]
    fn empty_orphan_warns_without_modifying_index_or_initializing() {
        let root = orphan();
        let repo = git2::Repository::open(root.path()).unwrap();
        assert!(repo.statuses(None).unwrap().is_empty());
        let before = fs::read(repo.path().join("index")).unwrap();
        let result = inspect_submodules(root.path()).unwrap();
        assert!(result.blockers.is_empty());
        assert_eq!(result.warnings.len(), 1);
        assert_eq!(result.warnings[0].path, "nested");
        assert_eq!(before, fs::read(repo.path().join("index")).unwrap());
        assert!(fs::read_dir(root.path().join("nested"))
            .unwrap()
            .next()
            .is_none());
        assert!(!repo.path().join("modules").exists());
        fs::remove_dir(root.path().join("nested")).unwrap();
        assert_eq!(
            inspect_submodules(root.path()).unwrap().warnings,
            result.warnings
        );
        fs::remove_file(root.path().join(".gitmodules")).unwrap();
        assert!(inspect_submodules(root.path()).unwrap().blockers.is_empty());
    }

    #[test]
    fn mapped_uninitialized_submodule_is_still_blocked() {
        let root = orphan();
        fs::write(
            root.path().join(".gitmodules"),
            "[submodule \"alias\"]\npath = nested\nurl = https://example.invalid/nested.git\n",
        )
        .unwrap();
        let result = inspect_submodules(root.path()).unwrap();
        assert_eq!(result.blockers.len(), 1);
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn local_content_and_hidden_git_data_are_blocked() {
        for name in ["work.txt", ".git", ".hidden", "node_modules"] {
            let root = orphan();
            fs::write(root.path().join("nested").join(name), "preserve this").unwrap();
            let result = inspect_submodules(root.path()).unwrap();
            assert_eq!(result.blockers.len(), 1, "{name}");
            assert!(result.warnings.is_empty());
        }
    }

    #[test]
    fn deinitialized_and_external_submodule_git_data_are_blocked() {
        let root = orphan();
        let repo = git2::Repository::open(root.path()).unwrap();
        fs::create_dir_all(repo.path().join("modules/arbitrary-alias/objects")).unwrap();
        assert_eq!(inspect_submodules(root.path()).unwrap().blockers.len(), 1);
        fs::remove_dir_all(repo.path().join("modules")).unwrap();
        repo.config()
            .unwrap()
            .set_str("submodule.alias.gitdir", "/external/module.git")
            .unwrap();
        assert_eq!(inspect_submodules(root.path()).unwrap().blockers.len(), 1);
    }

    #[test]
    fn malformed_gitmodules_is_not_treated_as_an_orphan() {
        let root = orphan();
        fs::write(root.path().join(".gitmodules"), "[broken\n").unwrap();
        assert!(inspect_submodules(root.path()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_cannot_hide_submodule_content_or_metadata() {
        use std::os::unix::fs::symlink;
        for target in ["nested", ".gitmodules"] {
            let root = orphan();
            let path = root.path().join(target);
            if path.is_dir() {
                fs::remove_dir(&path).unwrap();
            } else {
                fs::remove_file(&path).unwrap();
            }
            symlink(root.path().join("missing"), &path).unwrap();
            assert!(inspect_submodules(root.path()).is_err());
        }
    }

    #[test]
    fn ordinary_repository_has_no_submodule_notices() {
        let root = tempfile::tempdir().unwrap();
        git2::Repository::init(root.path()).unwrap();
        let result = inspect_submodules(root.path()).unwrap();
        assert!(result.blockers.is_empty() && result.warnings.is_empty());
    }
}
