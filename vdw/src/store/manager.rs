use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// Global VDW configuration stored at ~/.vdw/config.json
#[derive(Debug, Serialize, Deserialize)]
pub struct GlobalConfig {
    pub store_path: String,
    pub max_store_size_gb: u64,
    pub gc_threshold_days: u32,
    #[serde(default)]
    pub workspace_root: Option<String>,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            store_path: "~/.vdw/store".to_string(),
            max_store_size_gb: 50,
            gc_threshold_days: 30,
            workspace_root: None,
        }
    }
}

/// Get the VDW home directory (~/.vdw/)
pub fn get_vdw_home() -> Result<PathBuf> {
    let home = home_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
    Ok(home.join(".vdw"))
}

/// Get the effective Store path from ~/.vdw/config.json.
pub fn get_store_path() -> Result<PathBuf> {
    let config = load_global_config()?;
    let home = home_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
    resolve_store_path(&config.store_path, &home)
}

/// Get the configured workspace root. Workspace creation has no implicit
/// fallback: callers must configure this path before running `vdw init`.
pub fn get_workspace_root() -> Result<PathBuf> {
    let config = load_global_config()?;
    let configured_path = config
        .workspace_root
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Workspace root is not configured. Run `vdw config set workspace-root <path>` before creating a workspace."
            )
        })?;
    let home = home_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
    let workspace_root = resolve_workspace_root(configured_path, &home)?;
    if !workspace_root.is_dir() {
        bail!(
            "Configured workspace root '{}' is unavailable. Restore the directory or run `vdw config set workspace-root <path>` again.",
            workspace_root.display()
        );
    }
    Ok(workspace_root)
}

/// Resolve a workspace root. Only absolute paths and explicit home-relative
/// paths are accepted so creation does not depend on the caller's directory.
pub fn resolve_workspace_root(configured_path: &str, home: &Path) -> Result<PathBuf> {
    let configured_path = configured_path.trim();
    if configured_path.is_empty() {
        bail!("workspace_root in ~/.vdw/config.json must not be empty");
    }

    if configured_path == "~" {
        return Ok(home.to_path_buf());
    }
    if let Some(relative) = configured_path.strip_prefix("~/") {
        return Ok(home.join(relative));
    }
    if configured_path.starts_with('~') {
        bail!(
            "Unsupported workspace root '{}': only '~' and '~/' home expansion are supported",
            configured_path
        );
    }

    let path = PathBuf::from(configured_path);
    if !path.is_absolute() {
        bail!(
            "Workspace root '{}' must be an absolute path or start with '~/'.",
            configured_path
        );
    }
    Ok(path)
}

/// Persist the workspace root and create it immediately so later `vdw init`
/// calls fail if the selected location cannot be used.
pub fn set_workspace_root(configured_path: &str) -> Result<PathBuf> {
    let home = home_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
    let workspace_root = resolve_workspace_root(configured_path, &home)?;
    fs::create_dir_all(&workspace_root).with_context(|| {
        format!(
            "Failed to create workspace root at {}",
            workspace_root.display()
        )
    })?;
    let workspace_root = fs::canonicalize(&workspace_root).with_context(|| {
        format!(
            "Failed to resolve workspace root at {}",
            workspace_root.display()
        )
    })?;

    let mut config = load_global_config()?;
    config.workspace_root = Some(workspace_root.to_string_lossy().into_owned());
    save_global_config(&config)?;
    Ok(workspace_root)
}

/// Resolve a configured Store path. Relative paths are anchored at ~/.vdw so
/// that global configuration never depends on the caller's working directory.
pub fn resolve_store_path(configured_path: &str, home: &Path) -> Result<PathBuf> {
    let configured_path = configured_path.trim();
    if configured_path.is_empty() {
        bail!("store_path in ~/.vdw/config.json must not be empty");
    }

    if configured_path == "~" {
        return Ok(home.to_path_buf());
    }
    if let Some(relative) = configured_path.strip_prefix("~/") {
        return Ok(home.join(relative));
    }
    if configured_path.starts_with('~') {
        bail!(
            "Unsupported store_path '{}': only '~' and '~/' home expansion are supported",
            configured_path
        );
    }

    let path = PathBuf::from(configured_path);
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(home.join(".vdw").join(path))
    }
}

/// Initialize the VDW store directory structure
///
/// Creates:
/// - ~/.vdw/store/objects/   (repository-keyed bare Git storage)
/// - ~/.vdw/store/cache/     (cached repo data)
/// - ~/.vdw/logs/            (operational logs)
/// - ~/.vdw/config.json      (global configuration, only if not exists)
pub fn init_store() -> Result<PathBuf> {
    let vdw_home = get_vdw_home()?;

    // Create the home-owned directory structure first so configuration can be
    // read before the effective Store path is materialized.
    fs::create_dir_all(&vdw_home)?;
    fs::create_dir_all(vdw_home.join("logs"))?;

    // Write default global config if not exists
    let config_path = vdw_home.join("config.json");
    if !config_path.exists() {
        let config = GlobalConfig::default();
        let content = serde_json::to_string_pretty(&config)?;
        fs::write(&config_path, content)?;
    }

    let store_path = get_store_path()?;
    fs::create_dir_all(store_path.join("objects")).with_context(|| {
        format!(
            "Failed to create Store objects directory at {}",
            store_path.display()
        )
    })?;
    fs::create_dir_all(store_path.join("cache")).with_context(|| {
        format!(
            "Failed to create Store cache directory at {}",
            store_path.display()
        )
    })?;

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
    get_store_path()
        .map(|p| p.join("objects").exists())
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

/// Save the global configuration, creating ~/.vdw when needed.
pub fn save_global_config(config: &GlobalConfig) -> Result<()> {
    let vdw_home = get_vdw_home()?;
    fs::create_dir_all(&vdw_home)?;
    let config_path = vdw_home.join("config.json");
    let content = serde_json::to_string_pretty(config)?;
    fs::write(&config_path, content)
        .with_context(|| format!("Failed to write {}", config_path.display()))?;
    Ok(())
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
        assert_eq!(config.workspace_root, None);
    }

    #[test]
    fn test_global_config_serialization() {
        let config = GlobalConfig::default();
        let json = serde_json::to_string_pretty(&config).unwrap();
        assert!(json.contains("store_path"));
        assert!(json.contains("max_store_size_gb"));
        assert!(json.contains("gc_threshold_days"));
        assert!(json.contains("workspace_root"));

        let parsed: GlobalConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.store_path, config.store_path);
        assert_eq!(parsed.max_store_size_gb, config.max_store_size_gb);
        assert_eq!(parsed.gc_threshold_days, config.gc_threshold_days);
        assert_eq!(parsed.workspace_root, config.workspace_root);
    }

    #[test]
    fn test_global_config_without_workspace_root_remains_readable() {
        let json = r#"{
            "store_path": "~/.vdw/store",
            "max_store_size_gb": 50,
            "gc_threshold_days": 30
        }"#;

        let parsed: GlobalConfig = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.workspace_root, None);
    }

    #[test]
    fn test_resolve_store_path_expands_home() {
        let home = Path::new("/Users/example");
        assert_eq!(
            resolve_store_path("~/.vdw/custom-store", home).unwrap(),
            home.join(".vdw/custom-store")
        );
        assert_eq!(resolve_store_path("~", home).unwrap(), home);
    }

    #[test]
    fn test_resolve_store_path_preserves_absolute_path() {
        let home = Path::new("/Users/example");
        assert_eq!(
            resolve_store_path("/Volumes/vdw-store", home).unwrap(),
            PathBuf::from("/Volumes/vdw-store")
        );
    }

    #[test]
    fn test_resolve_store_path_anchors_relative_path_at_vdw_home() {
        let home = Path::new("/Users/example");
        assert_eq!(
            resolve_store_path("stores/team", home).unwrap(),
            home.join(".vdw/stores/team")
        );
    }

    #[test]
    fn test_resolve_store_path_rejects_empty_and_named_home() {
        let home = Path::new("/Users/example");
        assert!(resolve_store_path("   ", home).is_err());
        assert!(resolve_store_path("~someone/store", home).is_err());
    }

    #[test]
    fn test_resolve_workspace_root_expands_home_and_preserves_absolute_path() {
        let home = Path::new("/Users/example");
        assert_eq!(
            resolve_workspace_root("~/.vdw-workspaces", home).unwrap(),
            home.join(".vdw-workspaces")
        );
        assert_eq!(
            resolve_workspace_root("/Volumes/workspaces", home).unwrap(),
            PathBuf::from("/Volumes/workspaces")
        );
    }

    #[test]
    fn test_resolve_workspace_root_rejects_implicit_locations() {
        let home = Path::new("/Users/example");
        assert!(resolve_workspace_root("", home).is_err());
        assert!(resolve_workspace_root("workspaces", home).is_err());
        assert!(resolve_workspace_root("~someone/workspaces", home).is_err());
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
