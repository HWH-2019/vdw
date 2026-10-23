use anyhow::{bail, Context, Result};
use fs2::FileExt;
use serde::{de::DeserializeOwned, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);
pub fn id(prefix: &str) -> String {
    format!(
        "{}-{}",
        prefix,
        &hash(
            format!(
                "{}:{}:{}",
                chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            )
            .as_bytes()
        )[..24]
    )
}
pub fn hash(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}
pub fn hash_file(path: &Path) -> Result<String> {
    let mut f = File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}
pub fn private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
pub fn atomic_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("Missing parent")?;
    private_dir(parent)?;
    let tmp = parent.join(id(".pending"));
    let result = (|| {
        let mut opts = OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        fs::rename(&tmp, path)?;
        #[cfg(unix)]
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}
pub fn save<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    atomic_bytes(path, &serde_json::to_vec_pretty(value)?)
}
pub fn read<T: DeserializeOwned>(path: &Path) -> Result<T> {
    serde_json::from_slice(
        &fs::read(path).with_context(|| format!("Cannot read {}", path.display()))?,
    )
    .with_context(|| format!("Invalid JSON: {}", path.display()))
}
pub fn read_or<T: DeserializeOwned + Default>(path: &Path) -> Result<T> {
    if !path.exists() {
        Ok(T::default())
    } else {
        read(path)
    }
}
pub fn safe_relative(value: &str) -> Result<PathBuf> {
    let mut relative = PathBuf::new();
    for component in Path::new(value).components() {
        match component {
            Component::Normal(part) => relative.push(part),
            Component::CurDir => {}
            _ => bail!("Unsafe relative path: {}", value),
        }
    }
    if relative.as_os_str().is_empty() {
        bail!("Unsafe relative path: {}", value);
    }
    Ok(relative)
}
pub fn confined(root: &Path, relative: &str) -> Result<PathBuf> {
    let rel = safe_relative(relative)?;
    let mut path = root.to_path_buf();
    for component in rel.components() {
        path.push(component);
        if let Ok(meta) = fs::symlink_metadata(&path) {
            if meta.file_type().is_symlink() {
                bail!("Symlink boundary: {}", path.display());
            }
        }
    }
    Ok(path)
}
pub fn absolute(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        bail!("Path must be absolute: {}", path.display());
    }
    let mut result = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                bail!("Parent traversal is not allowed");
            }
            Component::CurDir => {}
            _ => result.push(part),
        }
    }
    // Reject a symlink anywhere, including an existing ancestor of a new target.
    let mut cursor = PathBuf::new();
    for part in result.components() {
        cursor.push(part);
        if fs::symlink_metadata(&cursor)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            // /var and /tmp are standard OS aliases on macOS, normalize those only.
            if cursor == Path::new("/tmp") || cursor == Path::new("/var") {
                cursor = fs::canonicalize(&cursor)?;
            } else {
                bail!("Symlink path is not allowed: {}", cursor.display());
            }
        }
    }
    Ok(cursor)
}
pub fn named_file(dir: &Path, name: &str) -> Result<PathBuf> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        bail!("Invalid identifier");
    }
    Ok(dir.join(format!("{name}.json")))
}
pub struct Guard {
    _file: File,
}
impl Guard {
    pub fn job(home: &Path, name: &str) -> Result<Self> {
        let path = named_file(&home.join("lifecycle/workers"), name)?.with_extension("lock");
        private_dir(path.parent().unwrap())?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        file.try_lock_exclusive()
            .context("Worker already running")?;
        Ok(Self { _file: file })
    }
    pub fn acquire(home: &Path) -> Result<Self> {
        let dir = home.join("lifecycle");
        private_dir(&dir)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(dir.join("engine.lock"))?;
        file.try_lock_exclusive()
            .context("Another VDW operation is running; retry after it completes")?;
        Ok(Self { _file: file })
    }
}
pub fn git_output(repo: &Path, args: &[&str]) -> Result<Output> {
    let output = Command::new("git")
        .arg("--no-optional-locks")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()?;
    if !output.status.success() {
        bail!(
            "Git operation failed in {}: {}",
            repo.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output)
}
pub fn git(repo: &Path, args: &[&str]) -> Result<String> {
    Ok(String::from_utf8(git_output(repo, args)?.stdout)?
        .trim()
        .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leading_dot_relative_paths_are_normalized() {
        for value in [
            "packages/repo",
            "./packages/repo",
            "././packages/repo",
            "packages/./repo",
            "./packages//repo/",
        ] {
            assert_eq!(safe_relative(value).unwrap(), Path::new("packages/repo"));
            assert_eq!(
                safe_relative(value).unwrap().to_str().unwrap(),
                Path::new("packages").join("repo").to_str().unwrap()
            );
        }
    }

    #[test]
    fn relative_paths_reject_roots_empty_targets_and_parent_traversal() {
        for value in [
            "",
            ".",
            "./",
            "././",
            "/packages/repo",
            "../repo",
            "./../repo",
            "packages/../repo",
            "./packages/../../repo",
        ] {
            assert!(safe_relative(value).is_err(), "accepted {value:?}");
        }
        #[cfg(windows)]
        for value in [r"C:\packages\repo", r"C:packages\repo", r"\packages\repo"] {
            assert!(safe_relative(value).is_err(), "accepted {value:?}");
        }
    }

    #[test]
    fn leading_dot_confined_paths_resolve_inside_the_root() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        fs::create_dir_all(root.join("packages/repo")).unwrap();
        assert_eq!(
            confined(&root, "./packages/repo").unwrap(),
            root.join("packages/repo")
        );
    }

    #[test]
    #[cfg(unix)]
    fn leading_dot_confined_paths_still_reject_symlink_boundaries() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        fs::create_dir(root.join("packages")).unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.join("packages/repo")).unwrap();
        let err = confined(&root, "./packages/repo/file").unwrap_err();
        assert!(err.to_string().contains("Symlink boundary"));
    }
}
