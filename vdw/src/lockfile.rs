use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::types::config::Dependency;

pub const LOCK_FILE_NAME: &str = ".vdw.lock";
pub const LOCK_FILE_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceLock {
    pub version: u32,
    pub dependencies: Vec<LockedDependency>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockedDependency {
    pub repo: String,
    pub path: String,
    pub editable: bool,
    pub revision: LockedRevision,
    pub resolved_commit: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockedRevision {
    pub kind: RevisionKind,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevisionKind {
    Branch,
    Tag,
    Commit,
}

impl WorkspaceLock {
    pub fn empty() -> Self {
        Self {
            version: LOCK_FILE_VERSION,
            dependencies: Vec::new(),
        }
    }

    pub fn path(workspace_root: &Path) -> PathBuf {
        workspace_root.join(LOCK_FILE_NAME)
    }

    pub fn load(workspace_root: &Path) -> Result<Option<Self>> {
        let path = Self::path(workspace_root);
        if !path.exists() {
            return Ok(None);
        }

        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let lock: Self = serde_yaml::from_str(&content)
            .with_context(|| format!("Failed to parse {}", path.display()))?;
        if lock.version != LOCK_FILE_VERSION {
            bail!(
                "Unsupported {} version {}; expected {}",
                LOCK_FILE_NAME,
                lock.version,
                LOCK_FILE_VERSION
            );
        }
        lock.validate()?;
        Ok(Some(lock))
    }

    pub fn load_required(workspace_root: &Path) -> Result<Self> {
        Self::load(workspace_root)?.with_context(|| {
            format!(
                "{} is required for frozen linking; run `vdw sync --update-lock` first",
                LOCK_FILE_NAME
            )
        })
    }

    pub fn save(&self, workspace_root: &Path) -> Result<()> {
        if self.version != LOCK_FILE_VERSION {
            bail!(
                "Refusing to write unsupported {} version {}",
                LOCK_FILE_NAME,
                self.version
            );
        }
        self.validate()?;

        let path = Self::path(workspace_root);
        let staging = workspace_root.join(format!("{}.tmp-{}", LOCK_FILE_NAME, std::process::id()));
        let mut canonical = self.clone();
        canonical
            .dependencies
            .sort_by(|left, right| left.path.cmp(&right.path));
        let content =
            serde_yaml::to_string(&canonical).context("Failed to serialize workspace lock")?;
        std::fs::write(&staging, content)
            .with_context(|| format!("Failed to write {}", staging.display()))?;

        #[cfg(windows)]
        if path.exists() {
            std::fs::remove_file(&path)
                .with_context(|| format!("Failed to replace {}", path.display()))?;
        }

        std::fs::rename(&staging, &path)
            .with_context(|| format!("Failed to publish {}", path.display()))?;
        Ok(())
    }

    pub fn entry_for_path(&self, path: &str) -> Option<&LockedDependency> {
        self.dependencies.iter().find(|entry| entry.path == path)
    }

    pub fn matching_entry(&self, dependency: &Dependency) -> Option<&LockedDependency> {
        self.entry_for_path(&dependency.path)
            .filter(|entry| entry.matches(dependency))
    }

    pub fn upsert(&mut self, entry: LockedDependency) {
        if let Some(existing) = self
            .dependencies
            .iter_mut()
            .find(|existing| existing.path == entry.path)
        {
            *existing = entry;
        } else {
            self.dependencies.push(entry);
        }
    }

    pub fn retain_paths<'a>(&mut self, paths: impl Iterator<Item = &'a str>) {
        let paths = paths.collect::<std::collections::HashSet<_>>();
        self.dependencies
            .retain(|entry| paths.contains(entry.path.as_str()));
    }

    fn validate(&self) -> Result<()> {
        let mut paths = HashSet::new();
        for dependency in &self.dependencies {
            if dependency.repo.is_empty() || dependency.path.is_empty() {
                bail!(
                    "{} contains an entry with an empty repo or path",
                    LOCK_FILE_NAME
                );
            }
            if !paths.insert(dependency.path.as_str()) {
                bail!(
                    "{} contains duplicate entries for '{}'",
                    LOCK_FILE_NAME,
                    dependency.path
                );
            }
            if dependency.revision.value.is_empty() {
                bail!(
                    "{} entry for '{}' has an empty revision",
                    LOCK_FILE_NAME,
                    dependency.path
                );
            }
            if dependency.resolved_commit.len() != 40
                || !dependency
                    .resolved_commit
                    .chars()
                    .all(|character| character.is_ascii_hexdigit())
            {
                bail!(
                    "{} entry for '{}' has an invalid resolved_commit",
                    LOCK_FILE_NAME,
                    dependency.path
                );
            }
        }
        Ok(())
    }
}

impl LockedDependency {
    pub fn from_dependency(
        dependency: &Dependency,
        resolved_commit: impl Into<String>,
    ) -> Result<Self> {
        Ok(Self {
            repo: dependency.dep_id().to_string(),
            path: dependency.path.clone(),
            editable: dependency.editable,
            revision: LockedRevision::from_dependency(dependency)?,
            resolved_commit: resolved_commit.into(),
        })
    }

    pub fn matches(&self, dependency: &Dependency) -> bool {
        self.repo == dependency.dep_id()
            && self.path == dependency.path
            && self.editable == dependency.editable
            && LockedRevision::from_dependency(dependency)
                .map(|revision| self.revision == revision)
                .unwrap_or(false)
    }
}

impl LockedRevision {
    pub fn from_dependency(dependency: &Dependency) -> Result<Self> {
        if let Some(branch) = &dependency.branch {
            return Ok(Self {
                kind: RevisionKind::Branch,
                value: branch.clone(),
            });
        }
        if let Some(tag) = &dependency.tag {
            return Ok(Self {
                kind: RevisionKind::Tag,
                value: tag.clone(),
            });
        }
        if let Some(commit) = &dependency.commit {
            return Ok(Self {
                kind: RevisionKind::Commit,
                value: commit.clone(),
            });
        }
        bail!(
            "Dependency '{}' does not declare a branch, tag, or commit",
            dependency.dep_id()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::config::Scope;

    fn dependency() -> Dependency {
        Dependency {
            remote: "https://github.com/example/service.git".to_string(),
            origin: String::new(),
            local: String::new(),
            repo: String::new(),
            path: "packages/service".to_string(),
            branch: Some("main".to_string()),
            tag: None,
            commit: None,
            scope: Scope {
                include: vec!["**/*".to_string()],
                exclude: vec![],
            },
            editable: false,
        }
    }

    #[test]
    fn lock_roundtrip_and_match() {
        let dir = tempfile::tempdir().unwrap();
        let dep = dependency();
        let entry = LockedDependency::from_dependency(&dep, "a".repeat(40)).unwrap();
        let mut lock = WorkspaceLock::empty();
        lock.upsert(entry.clone());
        lock.save(dir.path()).unwrap();

        let loaded = WorkspaceLock::load_required(dir.path()).unwrap();
        assert_eq!(loaded.dependencies, vec![entry]);
        assert!(loaded.matching_entry(&dep).is_some());
    }

    #[test]
    fn lock_rejects_config_drift_at_same_path() {
        let dep = dependency();
        let entry = LockedDependency::from_dependency(&dep, "a".repeat(40)).unwrap();
        let mut changed = dep.clone();
        changed.branch = Some("develop".to_string());

        assert!(!entry.matches(&changed));
    }

    #[test]
    fn retain_paths_removes_deleted_dependencies() {
        let dep = dependency();
        let mut lock = WorkspaceLock::empty();
        lock.upsert(LockedDependency::from_dependency(&dep, "a".repeat(40)).unwrap());

        lock.retain_paths(std::iter::empty());
        assert!(lock.dependencies.is_empty());
    }

    #[test]
    fn load_rejects_duplicate_paths() {
        let dir = tempfile::tempdir().unwrap();
        let entry = LockedDependency::from_dependency(&dependency(), "a".repeat(40)).unwrap();
        let lock = WorkspaceLock {
            version: LOCK_FILE_VERSION,
            dependencies: vec![entry.clone(), entry],
        };
        let content = serde_yaml::to_string(&lock).unwrap();
        std::fs::write(WorkspaceLock::path(dir.path()), content).unwrap();

        let error = WorkspaceLock::load_required(dir.path()).unwrap_err();
        assert!(error.to_string().contains("duplicate entries"));
    }

    #[test]
    fn save_orders_dependencies_by_path() {
        let dir = tempfile::tempdir().unwrap();
        let first = dependency();
        let mut second = dependency();
        second.path = "apps/web".to_string();
        let lock = WorkspaceLock {
            version: LOCK_FILE_VERSION,
            dependencies: vec![
                LockedDependency::from_dependency(&first, "a".repeat(40)).unwrap(),
                LockedDependency::from_dependency(&second, "b".repeat(40)).unwrap(),
            ],
        };

        lock.save(dir.path()).unwrap();
        let loaded = WorkspaceLock::load_required(dir.path()).unwrap();
        assert_eq!(loaded.dependencies[0].path, "apps/web");
        assert_eq!(loaded.dependencies[1].path, "packages/service");
    }
}
