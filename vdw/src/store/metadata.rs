use std::fs;
use std::path::Path;

use anyhow::Result;
use chrono::Utc;

use crate::types::store::StoreMetadata;

impl StoreMetadata {
    /// Load StoreMetadata from a metadata.json file in the given directory
    pub fn load(repo_path: &Path) -> Result<Self> {
        let metadata_file = repo_path.join("metadata.json");
        let content = fs::read_to_string(metadata_file)?;
        let metadata: StoreMetadata = serde_json::from_str(&content)?;
        Ok(metadata)
    }

    /// Save StoreMetadata to a metadata.json file in the given directory
    pub fn save(&self, repo_path: &Path) -> Result<()> {
        let metadata_file = repo_path.join("metadata.json");
        let content = serde_json::to_string_pretty(self)?;
        fs::write(metadata_file, content)?;
        Ok(())
    }

    /// Increment the reference count and update last_accessed timestamp
    pub fn increment_reference(&mut self) {
        self.reference_count += 1;
        self.last_accessed = Utc::now();
    }

    /// Decrement the reference count (never goes below zero)
    pub fn decrement_reference(&mut self) {
        if self.reference_count > 0 {
            self.reference_count -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_metadata() -> StoreMetadata {
        StoreMetadata {
            repo_url: "https://github.com/example/repo.git".to_string(),
            commit_hash: "abc123def456".to_string(),
            branch: Some("main".to_string()),
            tag: None,
            reference_count: 1,
            last_accessed: Utc::now(),
            created_at: Utc::now(),
            size_bytes: 1024,
            alternates_configured: true,
        }
    }

    #[test]
    fn test_metadata_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let original = sample_metadata();

        original.save(dir.path()).unwrap();
        let loaded = StoreMetadata::load(dir.path()).unwrap();

        assert_eq!(loaded.repo_url, original.repo_url);
        assert_eq!(loaded.commit_hash, original.commit_hash);
        assert_eq!(loaded.branch, original.branch);
        assert_eq!(loaded.tag, original.tag);
        assert_eq!(loaded.reference_count, original.reference_count);
        assert_eq!(loaded.size_bytes, original.size_bytes);
        assert_eq!(loaded.alternates_configured, original.alternates_configured);
    }

    #[test]
    fn test_metadata_json_serialization() {
        let metadata = sample_metadata();
        let json = serde_json::to_string_pretty(&metadata).unwrap();

        // Verify JSON contains expected fields
        assert!(json.contains("repo_url"));
        assert!(json.contains("commit_hash"));
        assert!(json.contains("reference_count"));
        assert!(json.contains("size_bytes"));
        assert!(json.contains("alternates_configured"));
    }

    #[test]
    fn test_metadata_deserialization_from_json() {
        let json = r#"{
            "repo_url": "https://github.com/test/repo.git",
            "commit_hash": "deadbeef",
            "branch": "develop",
            "tag": null,
            "reference_count": 3,
            "last_accessed": "2025-01-15T10:30:00Z",
            "created_at": "2025-01-10T08:00:00Z",
            "size_bytes": 2048,
            "alternates_configured": false
        }"#;

        let metadata: StoreMetadata = serde_json::from_str(json).unwrap();
        assert_eq!(metadata.repo_url, "https://github.com/test/repo.git");
        assert_eq!(metadata.commit_hash, "deadbeef");
        assert_eq!(metadata.branch, Some("develop".to_string()));
        assert_eq!(metadata.tag, None);
        assert_eq!(metadata.reference_count, 3);
        assert_eq!(metadata.size_bytes, 2048);
        assert!(!metadata.alternates_configured);
    }

    #[test]
    fn test_increment_reference() {
        let mut metadata = sample_metadata();
        assert_eq!(metadata.reference_count, 1);
        metadata.increment_reference();
        assert_eq!(metadata.reference_count, 2);
        metadata.increment_reference();
        assert_eq!(metadata.reference_count, 3);
    }

    #[test]
    fn test_decrement_reference() {
        let mut metadata = sample_metadata();
        assert_eq!(metadata.reference_count, 1);
        metadata.decrement_reference();
        assert_eq!(metadata.reference_count, 0);
    }

    #[test]
    fn test_decrement_reference_does_not_go_below_zero() {
        let mut metadata = sample_metadata();
        metadata.reference_count = 0;
        metadata.decrement_reference();
        assert_eq!(metadata.reference_count, 0);
    }

    #[test]
    fn test_load_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let result = StoreMetadata::load(dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_load_invalid_json() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("metadata.json"), "not valid json").unwrap();
        let result = StoreMetadata::load(dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_save_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let metadata = sample_metadata();

        let metadata_path = dir.path().join("metadata.json");
        assert!(!metadata_path.exists());

        metadata.save(dir.path()).unwrap();
        assert!(metadata_path.exists());

        let content = fs::read_to_string(metadata_path).unwrap();
        assert!(content.contains("repo_url"));
    }

    #[test]
    fn test_metadata_with_tag() {
        let dir = tempfile::tempdir().unwrap();
        let metadata = StoreMetadata {
            repo_url: "https://github.com/example/repo.git".to_string(),
            commit_hash: "abc123".to_string(),
            branch: None,
            tag: Some("v2.0".to_string()),
            reference_count: 0,
            last_accessed: Utc::now(),
            created_at: Utc::now(),
            size_bytes: 512,
            alternates_configured: false,
        };

        metadata.save(dir.path()).unwrap();
        let loaded = StoreMetadata::load(dir.path()).unwrap();

        assert_eq!(loaded.branch, None);
        assert_eq!(loaded.tag, Some("v2.0".to_string()));
    }
}
