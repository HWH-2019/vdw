#![deprecated(
    note = "Junction is no longer the default since v0.6.0. Windows now uses HardLink. This module will be removed in v0.7.0."
)]

use anyhow::Result;
use std::path::Path;

/// Normalize path separators to Windows backslash for cmd.exe compatibility.
/// Rust's Path on Windows preserves forward slashes if the path was constructed
/// from strings with `/` (e.g., from YAML config or Git Bash env vars).
/// cmd.exe's `mklink` does NOT understand forward slashes and treats `/xxx` as a switch.
#[cfg(windows)]
fn normalize_path_for_cmd(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\")
}

/// Create a junction point (Windows only)
#[cfg(windows)]
#[deprecated(note = "Use HardLink instead")]
pub fn create_junction(source: &Path, target: &Path) -> Result<()> {
    use std::process::Command;
    if target.exists() {
        return Err(anyhow::anyhow!(
            "Target already exists: {}",
            target.display()
        ));
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let target_str = normalize_path_for_cmd(target);
    let source_str = normalize_path_for_cmd(source);
    let output = Command::new("cmd")
        .args(&["/C", "mklink", "/J", &target_str, &source_str])
        .output()?;
    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "Junction creation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn create_junction(_source: &Path, _target: &Path) -> Result<()> {
    Err(anyhow::anyhow!(
        "Junction points are only supported on Windows"
    ))
}

/// Remove a junction point
#[cfg(windows)]
pub fn remove_junction(target: &Path) -> Result<()> {
    if target.exists() {
        std::fs::remove_dir(target)?;
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn remove_junction(_target: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_junction_on_non_windows() {
        #[cfg(not(windows))]
        {
            let dir = tempfile::tempdir().expect("Failed to create temp dir");
            let source = dir.path().join("source");
            let target = dir.path().join("target");
            let result = create_junction(&source, &target);
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("Windows"));
        }
    }

    #[test]
    fn test_remove_junction_nonexistent_is_ok() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let target = dir.path().join("nonexistent");
        assert!(remove_junction(&target).is_ok());
    }
}
