use std::path::Path;
use anyhow::Result;

/// Create a junction point (Windows only)
#[cfg(windows)]
pub fn create_junction(source: &Path, target: &Path) -> Result<()> {
    use std::process::Command;
    if target.exists() {
        return Err(anyhow::anyhow!("Target already exists: {}", target.display()));
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let output = Command::new("cmd")
        .args(&[
            "/C",
            "mklink",
            "/J",
            &target.to_string_lossy(),
            &source.to_string_lossy(),
        ])
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
