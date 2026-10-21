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
    let path = Path::new(value);
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|p| !matches!(p, Component::Normal(_)))
    {
        bail!("Unsafe relative path: {}", value);
    }
    Ok(path.into())
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
