use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Tracks the state of a linked workspace
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceState {
    /// Workspace root directory
    pub workspace_root: PathBuf,
    /// All link entries created by VDW
    pub links: Vec<LinkEntry>,
}

/// A single link entry (file or directory)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkEntry {
    /// Source path (in store or worktree)
    pub source: PathBuf,
    /// Target path (in workspace)
    pub target: PathBuf,
    /// Type of link
    pub link_type: LinkType,
    /// Which dependency this belongs to
    pub dependency_repo: String,
}

/// Type of filesystem link
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkType {
    HardLink,
    Symlink,
    Junction,
}

impl WorkspaceState {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self {
            workspace_root,
            links: Vec::new(),
        }
    }

    /// Load state from .vdw/state.json in workspace
    pub fn load(workspace_root: &std::path::Path) -> anyhow::Result<Self> {
        let state_path = workspace_root.join(".vdw").join("state.json");
        if !state_path.exists() {
            return Ok(Self::new(workspace_root.to_path_buf()));
        }
        let content = std::fs::read_to_string(&state_path)?;
        Ok(serde_json::from_str(&content)?)
    }

    /// Save state to .vdw/state.json
    pub fn save(&self) -> anyhow::Result<()> {
        let state_dir = self.workspace_root.join(".vdw");
        std::fs::create_dir_all(&state_dir)?;
        let state_path = state_dir.join("state.json");
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(state_path, content)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workspace_state_new() {
        let root = PathBuf::from("/tmp/test-workspace");
        let state = WorkspaceState::new(root.clone());
        assert_eq!(state.workspace_root, root);
        assert!(state.links.is_empty());
    }

    #[test]
    fn test_save_load_roundtrip() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let root = dir.path().to_path_buf();

        let mut state = WorkspaceState::new(root.clone());
        state.links.push(LinkEntry {
            source: PathBuf::from("/store/repo/src/main.rs"),
            target: PathBuf::from("/workspace/libs/repo/src/main.rs"),
            link_type: LinkType::HardLink,
            dependency_repo: "https://github.com/example/repo.git".to_string(),
        });

        state.save().expect("Failed to save state");

        let loaded = WorkspaceState::load(&root).expect("Failed to load state");
        assert_eq!(loaded.workspace_root, root);
        assert_eq!(loaded.links.len(), 1);
        assert_eq!(loaded.links[0].source, PathBuf::from("/store/repo/src/main.rs"));
        assert_eq!(loaded.links[0].target, PathBuf::from("/workspace/libs/repo/src/main.rs"));
        assert_eq!(loaded.links[0].link_type, LinkType::HardLink);
        assert_eq!(
            loaded.links[0].dependency_repo,
            "https://github.com/example/repo.git"
        );
    }

    #[test]
    fn test_load_missing_file_returns_empty_state() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let root = dir.path().to_path_buf();

        let state = WorkspaceState::load(dir.path()).expect("Failed to load missing state");
        assert_eq!(state.workspace_root, root);
        assert!(state.links.is_empty());
    }

    #[test]
    fn test_link_type_equality() {
        assert_eq!(LinkType::HardLink, LinkType::HardLink);
        assert_ne!(LinkType::HardLink, LinkType::Symlink);
        assert_ne!(LinkType::Symlink, LinkType::Junction);
    }

    #[test]
    fn test_save_creates_vdw_directory() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let root = dir.path().to_path_buf();

        let state = WorkspaceState::new(root);
        state.save().expect("Failed to save state");

        assert!(dir.path().join(".vdw").exists());
        assert!(dir.path().join(".vdw/state.json").exists());
    }

    #[test]
    fn test_link_entry_serialization() {
        let entry = LinkEntry {
            source: PathBuf::from("/store/repo/src/main.rs"),
            target: PathBuf::from("/workspace/libs/repo/src/main.rs"),
            link_type: LinkType::Symlink,
            dependency_repo: "https://github.com/example/repo.git".to_string(),
        };

        let json = serde_json::to_string(&entry).expect("Failed to serialize");
        let deserialized: LinkEntry = serde_json::from_str(&json).expect("Failed to deserialize");
        assert_eq!(deserialized.source, entry.source);
        assert_eq!(deserialized.target, entry.target);
        assert_eq!(deserialized.link_type, entry.link_type);
        assert_eq!(deserialized.dependency_repo, entry.dependency_repo);
    }
}
