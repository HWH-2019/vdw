use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Content Store metadata for a cached repository
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreMetadata {
    pub repo_url: String,
    pub commit_hash: String,
    pub branch: Option<String>,
    pub tag: Option<String>,
    pub reference_count: u32,
    pub last_accessed: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub size_bytes: u64,
    pub alternates_configured: bool,
}

/// Repository reference (for dependency graph)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepoRef {
    pub url: String,
    pub version: RepoVersion,
}

/// Version specifier for a repository reference
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RepoVersion {
    Branch(String),
    Tag(String),
    Commit(String),
}

impl RepoVersion {
    /// Get the version value as a string slice
    pub fn as_str(&self) -> &str {
        match self {
            RepoVersion::Branch(s) => s,
            RepoVersion::Tag(s) => s,
            RepoVersion::Commit(s) => s,
        }
    }
}

impl std::fmt::Display for RepoVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RepoVersion::Branch(s) => write!(f, "branch:{}", s),
            RepoVersion::Tag(s) => write!(f, "tag:{}", s),
            RepoVersion::Commit(s) => write!(f, "commit:{}", s),
        }
    }
}

impl std::fmt::Display for RepoRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}@{}", self.url, self.version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_repo_version_display() {
        assert_eq!(format!("{}", RepoVersion::Branch("main".to_string())), "branch:main");
        assert_eq!(format!("{}", RepoVersion::Tag("v1.0".to_string())), "tag:v1.0");
        assert_eq!(format!("{}", RepoVersion::Commit("abc123".to_string())), "commit:abc123");
    }

    #[test]
    fn test_repo_version_as_str() {
        assert_eq!(RepoVersion::Branch("main".to_string()).as_str(), "main");
        assert_eq!(RepoVersion::Tag("v1.0".to_string()).as_str(), "v1.0");
        assert_eq!(RepoVersion::Commit("abc123".to_string()).as_str(), "abc123");
    }

    #[test]
    fn test_repo_ref_display() {
        let repo_ref = RepoRef {
            url: "https://github.com/example/repo.git".to_string(),
            version: RepoVersion::Branch("main".to_string()),
        };
        assert_eq!(
            format!("{}", repo_ref),
            "https://github.com/example/repo.git@branch:main"
        );
    }

    #[test]
    fn test_repo_ref_equality() {
        let a = RepoRef {
            url: "https://github.com/example/repo.git".to_string(),
            version: RepoVersion::Branch("main".to_string()),
        };
        let b = RepoRef {
            url: "https://github.com/example/repo.git".to_string(),
            version: RepoVersion::Branch("main".to_string()),
        };
        assert_eq!(a, b);

        let c = RepoRef {
            url: "https://github.com/example/repo.git".to_string(),
            version: RepoVersion::Tag("v1.0".to_string()),
        };
        assert_ne!(a, c);
    }

    #[test]
    fn test_repo_version_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(RepoVersion::Branch("main".to_string()));
        set.insert(RepoVersion::Branch("main".to_string()));
        assert_eq!(set.len(), 1);
    }
}
