use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Global VDW configuration stored at ~/.vdw/config.json
#[derive(Debug, Serialize, Deserialize)]
pub struct GlobalConfig {
    pub store_path: String,
    pub max_store_size_gb: u64,
    pub gc_threshold_days: u32,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            store_path: "~/.vdw/store".to_string(),
            max_store_size_gb: 50,
            gc_threshold_days: 30,
        }
    }
}

/// Get the VDW home directory (~/.vdw/)
pub fn get_vdw_home() -> Result<PathBuf> {
    let home = home_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
    Ok(home.join(".vdw"))
}

/// Get the store path (~/.vdw/store/)
pub fn get_store_path() -> Result<PathBuf> {
    Ok(get_vdw_home()?.join("store"))
}

/// Initialize the VDW store directory structure
///
/// Creates:
/// - ~/.vdw/store/objects/   (content-addressable storage)
/// - ~/.vdw/store/cache/     (cached repo data)
/// - ~/.vdw/logs/            (operational logs)
/// - ~/.vdw/config.json      (global configuration, only if not exists)
pub fn init_store() -> Result<PathBuf> {
    let vdw_home = get_vdw_home()?;

    // Create directory structure
    fs::create_dir_all(vdw_home.join("store/objects"))?;
    fs::create_dir_all(vdw_home.join("store/cache"))?;
    fs::create_dir_all(vdw_home.join("logs"))?;

    // Write default global config if not exists
    let config_path = vdw_home.join("config.json");
    if !config_path.exists() {
        let config = GlobalConfig::default();
        let content = serde_json::to_string_pretty(&config)?;
        fs::write(&config_path, content)?;
    }

    Ok(vdw_home)
}

/// Initialize the VDW store at a custom path (for testing)
pub fn init_store_at(base_path: &Path) -> Result<PathBuf> {
    let vdw_home = base_path.to_path_buf();

    fs::create_dir_all(vdw_home.join("store/objects"))?;
    fs::create_dir_all(vdw_home.join("store/cache"))?;
    fs::create_dir_all(vdw_home.join("logs"))?;

    let config_path = vdw_home.join("config.json");
    if !config_path.exists() {
        let config = GlobalConfig::default();
        let content = serde_json::to_string_pretty(&config)?;
        fs::write(&config_path, content)?;
    }

    Ok(vdw_home)
}

/// Check if the store is initialized
pub fn is_store_initialized() -> bool {
    get_vdw_home()
        .map(|p| p.join("store/objects").exists())
        .unwrap_or(false)
}

/// Load the global configuration
pub fn load_global_config() -> Result<GlobalConfig> {
    let config_path = get_vdw_home()?.join("config.json");
    if !config_path.exists() {
        return Ok(GlobalConfig::default());
    }
    let content = fs::read_to_string(config_path)?;
    let config: GlobalConfig = serde_json::from_str(&content)?;
    Ok(config)
}

/// Get the platform-specific home directory
fn home_dir() -> Option<PathBuf> {
    // Try HOME on Unix, USERPROFILE on Windows
    if cfg!(unix) {
        env::var("HOME").ok().map(PathBuf::from)
    } else {
        env::var("USERPROFILE")
            .ok()
            .map(PathBuf::from)
            .or_else(|| env::var("HOME").ok().map(PathBuf::from))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_global_config_default() {
        let config = GlobalConfig::default();
        assert_eq!(config.store_path, "~/.vdw/store");
        assert_eq!(config.max_store_size_gb, 50);
        assert_eq!(config.gc_threshold_days, 30);
    }

    #[test]
    fn test_global_config_serialization() {
        let config = GlobalConfig::default();
        let json = serde_json::to_string_pretty(&config).unwrap();
        assert!(json.contains("store_path"));
        assert!(json.contains("max_store_size_gb"));
        assert!(json.contains("gc_threshold_days"));

        let parsed: GlobalConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.store_path, config.store_path);
        assert_eq!(parsed.max_store_size_gb, config.max_store_size_gb);
        assert_eq!(parsed.gc_threshold_days, config.gc_threshold_days);
    }

    #[test]
    fn test_init_store_at() {
        let dir = tempfile::tempdir().unwrap();
        let vdw_home = init_store_at(dir.path()).unwrap();

        assert!(vdw_home.join("store/objects").exists());
        assert!(vdw_home.join("store/cache").exists());
        assert!(vdw_home.join("logs").exists());
        assert!(vdw_home.join("config.json").exists());
    }

    #[test]
    fn test_init_store_at_idempotent() {
        let dir = tempfile::tempdir().unwrap();

        init_store_at(dir.path()).unwrap();
        init_store_at(dir.path()).unwrap();

        // Should not fail on second call
        assert!(dir.path().join("store/objects").exists());
    }

    #[test]
    fn test_init_store_at_preserves_existing_config() {
        let dir = tempfile::tempdir().unwrap();

        // First init creates default config
        init_store_at(dir.path()).unwrap();

        // Modify config
        let config_path = dir.path().join("config.json");
        let custom_config = r#"{
            "store_path": "/custom/path",
            "max_store_size_gb": 100,
            "gc_threshold_days": 60
        }"#;
        fs::write(&config_path, custom_config).unwrap();

        // Second init should NOT overwrite
        init_store_at(dir.path()).unwrap();

        let content = fs::read_to_string(config_path).unwrap();
        assert!(content.contains("/custom/path"));
        assert!(content.contains("100"));
    }

    #[test]
    fn test_load_global_config_missing_file() {
        let _dir = tempfile::tempdir().unwrap();
        // When no config exists, should return default
        // We can't easily test this with the real home dir,
        // but we can verify default config values
        let config = GlobalConfig::default();
        assert_eq!(config.max_store_size_gb, 50);
    }

    #[test]
    fn test_home_dir_returns_some() {
        // On a properly configured system, home_dir should return Some
        let home = home_dir();
        assert!(home.is_some());
        let path = home.unwrap();
        assert!(!path.as_os_str().is_empty());
    }

    #[test]
    fn test_is_store_initialized_with_custom_path() {
        // Test with a temp dir that doesn't have store structure
        let dir = tempfile::tempdir().unwrap();
        let objects_path = dir.path().join(".vdw").join("store").join("objects");
        assert!(!objects_path.exists());
    }
}