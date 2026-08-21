use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::ffi::OsStr;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[allow(deprecated)]
use crate::link::junction;
use crate::link::{hardlink, platform, symlink};
use crate::types::config::Scope;
use crate::types::workspace::{LinkEntry, LinkType};

/// Get the file index (inode) and volume serial number for a file.
/// On Windows: uses `GetFileInformationByHandle` to get `nFileIndexLow+High` and `dwVolumeSerialNumber`.
/// On Unix: uses metadata's `ino()` and `dev()`.
/// Returns `(inode, volume)` or `None` if not available.
pub(crate) fn get_file_inode(path: &Path) -> Option<(u64, u64)> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let Ok(meta) = std::fs::metadata(path) {
            return Some((meta.ino(), meta.dev()));
        }
    }
    #[cfg(windows)]
    {
        use std::fs::File;
        use std::os::windows::io::AsRawHandle;
        // Use Win32 GetFileInformationByHandle via external function
        #[repr(C)]
        struct BY_HANDLE_FILE_INFORMATION {
            file_attributes: u32,
            creation_time: u64,
            last_access_time: u64,
            last_write_time: u64,
            volume_serial_number: u32,
            file_size_high: u32,
            file_size_low: u32,
            n_number_of_links: u32,
            file_index_high: u32,
            file_index_low: u32,
        }

        extern "system" {
            fn GetFileInformationByHandle(
                h_file: *mut std::ffi::c_void,
                lp_file_information: *mut BY_HANDLE_FILE_INFORMATION,
            ) -> i32;
        }

        if let Ok(file) = File::open(path) {
            let handle = file.as_raw_handle();
            let mut info = BY_HANDLE_FILE_INFORMATION {
                file_attributes: 0,
                creation_time: 0,
                last_access_time: 0,
                last_write_time: 0,
                volume_serial_number: 0,
                file_size_high: 0,
                file_size_low: 0,
                n_number_of_links: 0,
                file_index_high: 0,
                file_index_low: 0,
            };
            unsafe {
                if GetFileInformationByHandle(handle as *mut std::ffi::c_void, &mut info) != 0 {
                    let inode = ((info.file_index_high as u64) << 32) | info.file_index_low as u64;
                    let volume = info.volume_serial_number as u64;
                    return Some((inode, volume));
                }
            }
        }
    }
    None
}

fn target_exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkConflictReason {
    ContentDiffers,
    MetadataDiffers,
    SourceMissing,
    TargetMissing,
    TargetUnreadable,
    TargetTypeChanged,
    LinkTargetChanged,
}

impl LinkConflictReason {
    pub fn description(self) -> &'static str {
        match self {
            Self::ContentDiffers => "workspace content differs from Store",
            Self::MetadataDiffers => "workspace file metadata differs from Store",
            Self::SourceMissing => "Store source is missing or unreadable",
            Self::TargetMissing => "workspace target is missing",
            Self::TargetUnreadable => "workspace target is unreadable",
            Self::TargetTypeChanged => "workspace target type changed",
            Self::LinkTargetChanged => "workspace link points to a different source",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkConflict {
    pub source: PathBuf,
    pub target: PathBuf,
    pub link_type: LinkType,
    pub dependency_repo: String,
    pub reason: LinkConflictReason,
    pub resolvable: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct LinkAudit {
    pub missing_count: usize,
    pub detached_equivalent_count: usize,
    pub conflicts: Vec<LinkConflict>,
}

enum LinkRemovalState {
    Missing,
    Owned,
    DetachedEquivalent,
    Conflict(LinkConflict),
}

fn conflict(entry: &LinkEntry, reason: LinkConflictReason, resolvable: bool) -> LinkRemovalState {
    LinkRemovalState::Conflict(LinkConflict {
        source: entry.source.clone(),
        target: entry.target.clone(),
        link_type: entry.link_type,
        dependency_repo: entry.dependency_repo.clone(),
        reason,
        resolvable,
    })
}

fn files_have_same_contents(source: &Path, target: &Path) -> Result<bool> {
    let source_metadata = std::fs::metadata(source)?;
    let target_metadata = std::fs::metadata(target)?;
    if source_metadata.len() != target_metadata.len() {
        return Ok(false);
    }

    let mut source_reader = BufReader::new(File::open(source)?);
    let mut target_reader = BufReader::new(File::open(target)?);
    let mut source_buffer = [0_u8; 64 * 1024];
    let mut target_buffer = [0_u8; 64 * 1024];

    loop {
        let source_count = source_reader.read(&mut source_buffer)?;
        let target_count = target_reader.read(&mut target_buffer)?;
        if source_count != target_count
            || source_buffer[..source_count] != target_buffer[..target_count]
        {
            return Ok(false);
        }
        if source_count == 0 {
            return Ok(true);
        }
    }
}

fn files_have_same_permissions(source: &Path, target: &Path) -> Result<bool> {
    let source_metadata = std::fs::metadata(source)?;
    let target_metadata = std::fs::metadata(target)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Ok(source_metadata.permissions().mode() == target_metadata.permissions().mode())
    }
    #[cfg(windows)]
    {
        Ok(source_metadata.permissions().readonly() == target_metadata.permissions().readonly())
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (source_metadata, target_metadata);
        Ok(true)
    }
}

fn hardlink_files_are_equivalent(source: &Path, target: &Path) -> Result<bool> {
    if !std::fs::symlink_metadata(source)?.file_type().is_file()
        || !std::fs::symlink_metadata(target)?.file_type().is_file()
    {
        return Ok(false);
    }
    Ok(files_have_same_contents(source, target)? && files_have_same_permissions(source, target)?)
}

pub(crate) fn regular_files_are_equivalent(source: &Path, target: &Path) -> Result<bool> {
    hardlink_files_are_equivalent(source, target)
}

fn canonical_link_target(target: &Path) -> Result<PathBuf> {
    let raw_target = std::fs::read_link(target)?;
    let resolved = if raw_target.is_absolute() {
        raw_target
    } else {
        target
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(raw_target)
    };
    Ok(std::fs::canonicalize(resolved)?)
}

fn inspect_link_for_removal(entry: &LinkEntry) -> LinkRemovalState {
    if !target_exists(&entry.target) {
        if entry.link_type == LinkType::HardLink
            && std::fs::symlink_metadata(&entry.source)
                .map(|metadata| metadata.file_type().is_file())
                .unwrap_or(false)
        {
            return conflict(entry, LinkConflictReason::TargetMissing, true);
        }
        return LinkRemovalState::Missing;
    }

    match entry.link_type {
        LinkType::HardLink => {
            let source_metadata = match std::fs::symlink_metadata(&entry.source) {
                Ok(metadata) if metadata.file_type().is_file() => metadata,
                _ => {
                    let resolvable = std::fs::symlink_metadata(&entry.target)
                        .map(|metadata| metadata.file_type().is_file())
                        .unwrap_or(false);
                    return conflict(entry, LinkConflictReason::SourceMissing, resolvable);
                }
            };
            let target_metadata = match std::fs::symlink_metadata(&entry.target) {
                Ok(metadata) if metadata.file_type().is_file() => metadata,
                Ok(_) => return conflict(entry, LinkConflictReason::TargetTypeChanged, false),
                Err(_) => return conflict(entry, LinkConflictReason::TargetUnreadable, false),
            };

            let source_identity = match get_file_inode(&entry.source) {
                Some(identity) => identity,
                None => return conflict(entry, LinkConflictReason::SourceMissing, false),
            };
            let target_identity = match get_file_inode(&entry.target) {
                Some(identity) => identity,
                None => return conflict(entry, LinkConflictReason::TargetUnreadable, false),
            };

            if source_identity == target_identity {
                return LinkRemovalState::Owned;
            }

            let resolvable = source_metadata.is_file() && target_metadata.is_file();
            match files_have_same_contents(&entry.source, &entry.target) {
                Ok(false) => conflict(entry, LinkConflictReason::ContentDiffers, resolvable),
                Err(_) => conflict(entry, LinkConflictReason::TargetUnreadable, false),
                Ok(true) => match files_have_same_permissions(&entry.source, &entry.target) {
                    Ok(true) => LinkRemovalState::DetachedEquivalent,
                    Ok(false) => conflict(entry, LinkConflictReason::MetadataDiffers, resolvable),
                    Err(_) => conflict(entry, LinkConflictReason::TargetUnreadable, false),
                },
            }
        }
        LinkType::Symlink => {
            let metadata = match std::fs::symlink_metadata(&entry.target) {
                Ok(metadata) => metadata,
                Err(_) => return conflict(entry, LinkConflictReason::TargetUnreadable, false),
            };
            if !metadata.file_type().is_symlink() {
                return conflict(entry, LinkConflictReason::TargetTypeChanged, false);
            }
            let actual_source = match canonical_link_target(&entry.target) {
                Ok(source) => source,
                Err(_) => return conflict(entry, LinkConflictReason::LinkTargetChanged, false),
            };
            let expected_source = match std::fs::canonicalize(&entry.source) {
                Ok(source) => source,
                Err(_) => return conflict(entry, LinkConflictReason::SourceMissing, false),
            };
            if actual_source != expected_source {
                return conflict(entry, LinkConflictReason::LinkTargetChanged, false);
            }
            LinkRemovalState::Owned
        }
        LinkType::Junction => {
            let actual_source = match std::fs::canonicalize(&entry.target) {
                Ok(source) => source,
                Err(_) => return conflict(entry, LinkConflictReason::TargetUnreadable, false),
            };
            let expected_source = match std::fs::canonicalize(&entry.source) {
                Ok(source) => source,
                Err(_) => return conflict(entry, LinkConflictReason::SourceMissing, false),
            };
            if actual_source != expected_source {
                return conflict(entry, LinkConflictReason::LinkTargetChanged, false);
            }
            LinkRemovalState::Owned
        }
    }
}

fn matching_existing_link_type(
    source: &Path,
    target: &Path,
    strategy: platform::LinkStrategy,
) -> Option<LinkType> {
    match strategy {
        platform::LinkStrategy::HardLink => {
            if !std::fs::symlink_metadata(source)
                .ok()?
                .file_type()
                .is_file()
                || !std::fs::symlink_metadata(target)
                    .ok()?
                    .file_type()
                    .is_file()
            {
                return None;
            }
            (get_file_inode(source)? == get_file_inode(target)?).then_some(LinkType::HardLink)
        }
        platform::LinkStrategy::Symlink => {
            let metadata = std::fs::symlink_metadata(target).ok()?;
            if !metadata.file_type().is_symlink() {
                return None;
            }
            let actual_source = canonical_link_target(target).ok()?;
            let expected_source = std::fs::canonicalize(source).ok()?;
            (actual_source == expected_source).then_some(LinkType::Symlink)
        }
        platform::LinkStrategy::Junction => {
            let actual_source = std::fs::canonicalize(target).ok()?;
            let expected_source = std::fs::canonicalize(source).ok()?;
            (actual_source == expected_source).then_some(LinkType::Junction)
        }
    }
}

/// Fail closed before unlink/relink if a tracked target has become an
/// independent file or points somewhere else. This protects files produced by
/// editors that implement atomic save by replacing a hardlink with a new inode.
pub fn verify_links_for_removal(entries: &[LinkEntry]) -> Result<()> {
    let audit = audit_links(entries);
    let blocking_conflicts = audit
        .conflicts
        .iter()
        .filter(|conflict| conflict.reason != LinkConflictReason::TargetMissing)
        .collect::<Vec<_>>();

    if !blocking_conflicts.is_empty() {
        const MAX_CONFLICTS_TO_PRINT: usize = 20;
        let preview = blocking_conflicts
            .iter()
            .take(MAX_CONFLICTS_TO_PRINT)
            .map(|conflict| {
                format!(
                    "{}: {}",
                    conflict.target.display(),
                    conflict.reason.description()
                )
            })
            .collect::<Vec<_>>()
            .join("\n  - ");
        let omitted = blocking_conflicts
            .len()
            .saturating_sub(MAX_CONFLICTS_TO_PRINT);
        let omitted_message = if omitted > 0 {
            format!("\n  ... and {} more conflict(s)", omitted)
        } else {
            String::new()
        };
        bail!(
            "Refusing to remove {} target(s) with workspace changes:\n  - {}{}\n\
             Run `vdw conflicts list` to inspect them, then resolve by accepting Store or workspace.",
            blocking_conflicts.len(),
            preview,
            omitted_message
        );
    }

    Ok(())
}

pub fn audit_links(entries: &[LinkEntry]) -> LinkAudit {
    let mut audit = LinkAudit::default();
    for entry in entries {
        match inspect_link_for_removal(entry) {
            LinkRemovalState::DetachedEquivalent => audit.detached_equivalent_count += 1,
            LinkRemovalState::Conflict(conflict) => audit.conflicts.push(conflict),
            LinkRemovalState::Missing => audit.missing_count += 1,
            LinkRemovalState::Owned => {}
        }
    }
    audit
}

/// Create links for a single dependency from source_dir to target_dir
/// Only links files that match the scope
pub fn create_dependency_links(
    source_dir: &Path,
    target_dir: &Path,
    scope: &Scope,
    dep_repo: &str,
) -> Result<Vec<LinkEntry>> {
    let mut entries = Vec::new();

    if !source_dir.exists() {
        return Err(anyhow::anyhow!(
            "Source directory does not exist: {}",
            source_dir.display()
        ));
    }

    // Determine link strategy ONCE for the entire dependency
    // (avoids per-file probing that causes random hardlink/symlink inconsistency)
    let base_strategy = platform::determine_strategy(source_dir, target_dir);

    // Walk the source directory and link files matching scope
    let walker = WalkDir::new(source_dir)
        .into_iter()
        .filter_entry(|entry| entry.depth() == 0 || entry.file_name() != OsStr::new(".git"));
    for entry in walker.filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }

        let rel_path = entry.path().strip_prefix(source_dir)?;
        let rel_str = rel_path.to_string_lossy().replace('\\', "/");

        // Check if file matches scope
        if !scope.matches(&rel_str) {
            continue;
        }

        let source_path = entry.path().to_path_buf();
        let target_path = target_dir.join(rel_path);

        // Select strategy for this specific file (usually base_strategy, but dirs get symlink)
        let strategy = platform::select_strategy_for_file(&source_path, base_strategy);

        let actual_link_type = if target_exists(&target_path) {
            match matching_existing_link_type(&source_path, &target_path, strategy) {
                Some(link_type) => link_type,
                None => {
                    if strategy == platform::LinkStrategy::HardLink
                        && hardlink_files_are_equivalent(&source_path, &target_path)
                            .unwrap_or(false)
                    {
                        match create_single_link(&source_path, &target_path, strategy) {
                            Ok(link_type) => link_type,
                            Err(error) => {
                                let _ = remove_links(&entries);
                                return Err(error).with_context(|| {
                                    format!(
                                        "Failed to restore equivalent workspace link {}",
                                        target_path.display()
                                    )
                                });
                            }
                        }
                    } else {
                        let _ = remove_links(&entries);
                        bail!(
                            "Refusing to overwrite an untracked workspace file: {}",
                            target_path.display()
                        );
                    }
                }
            }
        } else {
            match create_single_link(&source_path, &target_path, strategy) {
                Ok(link_type) => link_type,
                Err(error) => {
                    // Do not leave a half-linked dependency if a later file fails.
                    let _ = remove_links(&entries);
                    return Err(error).with_context(|| {
                        format!("Failed to create workspace link {}", target_path.display())
                    });
                }
            }
        };

        // Capture inode metadata for HardLink (used for broken link detection)
        let (source_inode, source_volume) = if actual_link_type == LinkType::HardLink {
            get_file_inode(&source_path).unwrap_or((0, 0))
        } else {
            (0, 0)
        };

        entries.push(LinkEntry {
            source: source_path,
            target: target_path,
            link_type: actual_link_type,
            dependency_repo: dep_repo.to_string(),
            source_inode: if source_inode != 0 {
                Some(source_inode)
            } else {
                None
            },
            source_volume: if source_volume != 0 {
                Some(source_volume)
            } else {
                None
            },
        });
    }

    Ok(entries)
}

/// Create a single link using the given strategy.
/// Returns the ACTUAL LinkType used (may differ from strategy if fallback was triggered).
fn create_single_link(
    source: &Path,
    target: &Path,
    strategy: platform::LinkStrategy,
) -> Result<LinkType> {
    let result = match strategy {
        platform::LinkStrategy::HardLink => {
            hardlink::create_hardlink(source, target).map(|_| LinkType::HardLink)
        }
        platform::LinkStrategy::Symlink => {
            symlink::create_symlink(source, target).map(|_| LinkType::Symlink)
        }
        platform::LinkStrategy::Junction =>
        {
            #[allow(deprecated)]
            junction::create_junction(source, target).map(|_| LinkType::Junction)
        }
    };

    match result {
        Ok(link_type) => Ok(link_type),
        Err(e) => {
            // On Windows, if symlink/junction fails due to missing privilege (error 1314),
            // check if privilege elevation is possible and retry.
            #[cfg(windows)]
            {
                let is_privilege_error = e
                    .downcast_ref::<std::io::Error>()
                    .map(|ioe| ioe.raw_os_error() == Some(1314))
                    .unwrap_or(false);

                if is_privilege_error && platform::try_enable_symlink_privilege() {
                    // Retry after privilege adjustment
                    return match strategy {
                        platform::LinkStrategy::Symlink => {
                            symlink::create_symlink(source, target)?;
                            Ok(LinkType::Symlink)
                        }
                        platform::LinkStrategy::Junction => {
                            #[allow(deprecated)]
                            junction::create_junction(source, target)?;
                            Ok(LinkType::Junction)
                        }
                        platform::LinkStrategy::HardLink => Err(e),
                    };
                }
            }

            Err(e)
        }
    }
}

/// Remove all links tracked in the given entries list
pub fn remove_links(entries: &[LinkEntry]) -> Result<()> {
    for entry in entries {
        match entry.link_type {
            LinkType::HardLink => hardlink::remove_hardlink(&entry.target)?,
            LinkType::Symlink => symlink::remove_symlink(&entry.target)?,
            LinkType::Junction =>
            {
                #[allow(deprecated)]
                junction::remove_junction(&entry.target)?
            }
        }
    }

    Ok(())
}

/// Restore a previously verified set of links without overwriting any target
/// that appeared after the operation started.
pub fn restore_links(entries: &[LinkEntry]) -> Result<()> {
    for entry in entries {
        if target_exists(&entry.target) {
            match inspect_link_for_removal(entry) {
                LinkRemovalState::Owned => continue,
                LinkRemovalState::DetachedEquivalent => match entry.link_type {
                    LinkType::HardLink => hardlink::create_hardlink(&entry.source, &entry.target)?,
                    LinkType::Symlink | LinkType::Junction => unreachable!(),
                },
                LinkRemovalState::Missing => {}
                LinkRemovalState::Conflict(conflict) => {
                    bail!(
                        "Refusing to overwrite a file while rolling back: {}: {}",
                        entry.target.display(),
                        conflict.reason.description()
                    );
                }
            }
            continue;
        }

        match entry.link_type {
            LinkType::HardLink => hardlink::create_hardlink(&entry.source, &entry.target)?,
            LinkType::Symlink => symlink::create_symlink(&entry.source, &entry.target)?,
            LinkType::Junction =>
            {
                #[allow(deprecated)]
                junction::create_junction(&entry.source, &entry.target)?
            }
        }
    }
    Ok(())
}

/// Remove empty parent directories up to (but not including) stop_at
pub fn cleanup_empty_dirs(path: &Path, stop_at: &Path) -> Result<()> {
    let mut current = path.to_path_buf();
    while current != stop_at && current.starts_with(stop_at) {
        if current.is_dir() && std::fs::read_dir(&current)?.next().is_none() {
            std::fs::remove_dir(&current)?;
        } else {
            break;
        }
        current = current.parent().unwrap_or(stop_at).to_path_buf();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_source_dir(dir: &std::path::Path) {
        // Create a directory structure:
        //   src/main.rs
        //   src/lib.rs
        //   src/utils/mod.rs
        //   tests/integration.rs
        //   README.md
        let src = dir.join("src");
        let utils = src.join("utils");
        let tests = dir.join("tests");
        std::fs::create_dir_all(&utils).expect("Failed to create utils dir");
        std::fs::create_dir_all(&tests).expect("Failed to create tests dir");

        std::fs::write(src.join("main.rs"), "fn main() {}").expect("Failed to write main.rs");
        std::fs::write(src.join("lib.rs"), "pub fn lib() {}").expect("Failed to write lib.rs");
        std::fs::write(utils.join("mod.rs"), "pub mod utils;").expect("Failed to write mod.rs");
        std::fs::write(tests.join("integration.rs"), "#[test] fn test() {}")
            .expect("Failed to write integration.rs");
        std::fs::write(dir.join("README.md"), "# Test Repo").expect("Failed to write README.md");
    }

    #[test]
    fn test_create_dependency_links_with_scope() {
        let source_dir = tempfile::tempdir().expect("Failed to create source temp dir");
        let target_dir = tempfile::tempdir().expect("Failed to create target temp dir");

        setup_source_dir(source_dir.path());

        let scope = Scope {
            include: vec!["src/**/*.rs".to_string()],
            exclude: vec![],
        };

        let entries = create_dependency_links(
            source_dir.path(),
            target_dir.path(),
            &scope,
            "https://github.com/example/repo.git",
        )
        .expect("Failed to create dependency links");

        // Should link: src/main.rs, src/lib.rs, src/utils/mod.rs
        assert_eq!(entries.len(), 3);

        // All entries should belong to the same repo
        for entry in &entries {
            assert_eq!(entry.dependency_repo, "https://github.com/example/repo.git");
        }

        // Check files exist in target
        assert!(target_dir.path().join("src/main.rs").exists());
        assert!(target_dir.path().join("src/lib.rs").exists());
        assert!(target_dir.path().join("src/utils/mod.rs").exists());
        // These should NOT be linked (don't match scope)
        assert!(!target_dir.path().join("tests/integration.rs").exists());
        assert!(!target_dir.path().join("README.md").exists());
    }

    #[test]
    fn test_create_dependency_links_with_exclude() {
        let source_dir = tempfile::tempdir().expect("Failed to create source temp dir");
        let target_dir = tempfile::tempdir().expect("Failed to create target temp dir");

        setup_source_dir(source_dir.path());

        let scope = Scope {
            include: vec!["src/**/*.rs".to_string()],
            exclude: vec!["src/**/mod.rs".to_string()],
        };

        let entries = create_dependency_links(
            source_dir.path(),
            target_dir.path(),
            &scope,
            "https://github.com/example/repo.git",
        )
        .expect("Failed to create dependency links");

        // Should link: src/main.rs, src/lib.rs (mod.rs excluded)
        assert_eq!(entries.len(), 2);
        assert!(target_dir.path().join("src/main.rs").exists());
        assert!(target_dir.path().join("src/lib.rs").exists());
        assert!(!target_dir.path().join("src/utils/mod.rs").exists());
    }

    #[test]
    fn test_create_dependency_links_never_projects_git_metadata() {
        let source_dir = tempfile::tempdir().expect("Failed to create source temp dir");
        let target_dir = tempfile::tempdir().expect("Failed to create target temp dir");
        setup_source_dir(source_dir.path());
        std::fs::create_dir_all(source_dir.path().join(".git/objects"))
            .expect("Failed to create .git directory");
        std::fs::write(source_dir.path().join(".git/HEAD"), "ref: refs/heads/main")
            .expect("Failed to write .git/HEAD");
        std::fs::write(
            source_dir.path().join(".git/objects/object"),
            "git metadata",
        )
        .expect("Failed to write git object");
        let scope = Scope {
            include: vec!["**/*".to_string()],
            exclude: vec![],
        };

        let entries = create_dependency_links(source_dir.path(), target_dir.path(), &scope, "repo")
            .expect("Failed to create dependency links");

        assert!(entries.iter().all(|entry| !entry.target.ends_with(".git")));
        assert!(!target_dir.path().join(".git").exists());
    }

    #[test]
    fn test_create_dependency_links_never_records_worktree_git_pointer() {
        let source_dir = tempfile::tempdir().expect("Failed to create source temp dir");
        let target_dir = tempfile::tempdir().expect("Failed to create target temp dir");
        setup_source_dir(source_dir.path());
        std::fs::write(
            source_dir.path().join(".git"),
            "gitdir: /store/repo/bare.git/worktrees/main\n",
        )
        .expect("Failed to write worktree .git pointer");
        let scope = Scope {
            include: vec!["**/*".to_string()],
            exclude: vec![],
        };

        let entries = create_dependency_links(source_dir.path(), target_dir.path(), &scope, "repo")
            .expect("Failed to create dependency links");

        assert!(entries.iter().all(|entry| !entry.target.ends_with(".git")));
        assert!(!target_dir.path().join(".git").exists());
    }

    #[test]
    fn test_create_dependency_links_missing_source() {
        let target_dir = tempfile::tempdir().expect("Failed to create target temp dir");

        let scope = Scope {
            include: vec!["**/*".to_string()],
            exclude: vec![],
        };

        let result = create_dependency_links(
            Path::new("/nonexistent/path"),
            target_dir.path(),
            &scope,
            "test",
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("does not exist"));
    }

    #[test]
    fn test_create_dependency_links_preserves_independent_existing_target() {
        let source_dir = tempfile::tempdir().expect("Failed to create source temp dir");
        let target_dir = tempfile::tempdir().expect("Failed to create target temp dir");
        setup_source_dir(source_dir.path());
        std::fs::create_dir_all(target_dir.path().join("src"))
            .expect("Failed to create target src dir");
        let protected_target = target_dir.path().join("src/main.rs");
        std::fs::write(&protected_target, "independent workspace edit")
            .expect("Failed to write independent target");
        let scope = Scope {
            include: vec!["src/**/*.rs".to_string()],
            exclude: vec![],
        };

        let error = create_dependency_links(source_dir.path(), target_dir.path(), &scope, "repo")
            .expect_err("Independent target must not be overwritten");

        assert!(error.to_string().contains("Refusing to overwrite"));
        assert_eq!(
            std::fs::read_to_string(protected_target).expect("Protected target was removed"),
            "independent workspace edit"
        );
        assert!(!target_dir.path().join("src/lib.rs").exists());
        assert!(!target_dir.path().join("src/utils/mod.rs").exists());
    }

    #[test]
    fn test_create_dependency_links_repairs_equivalent_existing_target() {
        let source_dir = tempfile::tempdir().expect("Failed to create source temp dir");
        let target_dir = tempfile::tempdir().expect("Failed to create target temp dir");
        setup_source_dir(source_dir.path());
        std::fs::create_dir_all(target_dir.path().join("src"))
            .expect("Failed to create target src dir");
        let source = source_dir.path().join("src/main.rs");
        let target = target_dir.path().join("src/main.rs");
        std::fs::copy(&source, &target).expect("Failed to create equivalent target");
        assert_ne!(get_file_inode(&source), get_file_inode(&target));
        let scope = Scope {
            include: vec!["src/main.rs".to_string()],
            exclude: vec![],
        };

        create_dependency_links(source_dir.path(), target_dir.path(), &scope, "repo")
            .expect("Equivalent target should be repaired");

        assert_eq!(get_file_inode(&source), get_file_inode(&target));
    }

    #[test]
    fn test_remove_links() {
        let source_dir = tempfile::tempdir().expect("Failed to create source temp dir");
        let target_dir = tempfile::tempdir().expect("Failed to create target temp dir");

        setup_source_dir(source_dir.path());

        let scope = Scope {
            include: vec!["src/**/*.rs".to_string()],
            exclude: vec![],
        };

        let entries = create_dependency_links(
            source_dir.path(),
            target_dir.path(),
            &scope,
            "https://github.com/example/repo.git",
        )
        .expect("Failed to create links");

        // Verify files exist
        assert!(target_dir.path().join("src/main.rs").exists());

        // Remove all links
        remove_links(&entries).expect("Failed to remove links");

        // Verify files are gone
        assert!(!target_dir.path().join("src/main.rs").exists());
        assert!(!target_dir.path().join("src/lib.rs").exists());
        assert!(!target_dir.path().join("src/utils/mod.rs").exists());
    }

    #[test]
    fn test_verify_valid_hardlink_for_removal() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let source = dir.path().join("source.txt");
        let target = dir.path().join("target.txt");
        std::fs::write(&source, "managed").expect("Failed to write source");
        std::fs::hard_link(&source, &target).expect("Failed to create hardlink");
        let (inode, volume) = get_file_inode(&source).expect("Missing file identity");
        let entry = LinkEntry {
            source,
            target,
            link_type: LinkType::HardLink,
            dependency_repo: "repo".to_string(),
            source_inode: Some(inode),
            source_volume: Some(volume),
        };

        verify_links_for_removal(&[entry]).expect("Valid hardlink must be removable");
    }

    #[test]
    fn test_verify_rejects_atomic_save_replacement_without_deleting_it() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let source = dir.path().join("source.txt");
        let target = dir.path().join("target.txt");
        std::fs::write(&source, "managed").expect("Failed to write source");
        std::fs::hard_link(&source, &target).expect("Failed to create hardlink");
        let (inode, volume) = get_file_inode(&source).expect("Missing file identity");

        std::fs::remove_file(&target).expect("Failed to replace target");
        std::fs::write(&target, "independent edit").expect("Failed to write replacement");
        let entry = LinkEntry {
            source,
            target: target.clone(),
            link_type: LinkType::HardLink,
            dependency_repo: "repo".to_string(),
            source_inode: Some(inode),
            source_volume: Some(volume),
        };

        let error = verify_links_for_removal(&[entry]).expect_err("Replacement must be rejected");
        assert!(error.to_string().contains("workspace changes"));
        assert_eq!(
            std::fs::read_to_string(target).expect("Replacement was deleted"),
            "independent edit"
        );
    }

    #[test]
    fn test_verify_allows_equivalent_atomic_save_replacement() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let source = dir.path().join("source.txt");
        let target = dir.path().join("target.txt");
        std::fs::write(&source, "managed").expect("Failed to write source");
        std::fs::hard_link(&source, &target).expect("Failed to create hardlink");
        let (inode, volume) = get_file_inode(&source).expect("Missing file identity");
        std::fs::remove_file(&target).expect("Failed to replace target");
        std::fs::copy(&source, &target).expect("Failed to copy equivalent replacement");
        let entry = LinkEntry {
            source,
            target,
            link_type: LinkType::HardLink,
            dependency_repo: "repo".to_string(),
            source_inode: Some(inode),
            source_volume: Some(volume),
        };

        let audit = audit_links(&[entry.clone()]);
        assert_eq!(audit.detached_equivalent_count, 1);
        assert!(audit.conflicts.is_empty());
        verify_links_for_removal(&[entry]).expect("Equivalent replacement must be removable");
    }

    #[test]
    fn test_audit_reports_only_real_content_conflicts() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let source = dir.path().join("source.txt");
        let equivalent = dir.path().join("equivalent.txt");
        let changed = dir.path().join("changed.txt");
        std::fs::write(&source, "managed").expect("Failed to write source");
        std::fs::copy(&source, &equivalent).expect("Failed to copy equivalent file");
        std::fs::write(&changed, "workspace edit").expect("Failed to write changed file");
        let entries = [equivalent, changed].map(|target| LinkEntry {
            source: source.clone(),
            target,
            link_type: LinkType::HardLink,
            dependency_repo: "repo".to_string(),
            source_inode: None,
            source_volume: None,
        });

        let audit = audit_links(&entries);

        assert_eq!(audit.detached_equivalent_count, 1);
        assert_eq!(audit.conflicts.len(), 1);
        assert_eq!(
            audit.conflicts[0].reason,
            LinkConflictReason::ContentDiffers
        );
    }

    #[test]
    fn test_verify_allows_already_missing_target() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let source = dir.path().join("source.txt");
        std::fs::write(&source, "store file").expect("source");
        let entry = LinkEntry {
            source,
            target: dir.path().join("missing.txt"),
            link_type: LinkType::HardLink,
            dependency_repo: "repo".to_string(),
            source_inode: None,
            source_volume: None,
        };

        verify_links_for_removal(&[entry]).expect("Missing target needs no deletion");
    }

    #[test]
    fn test_audit_exposes_missing_workspace_target_as_resolvable_conflict() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let source = dir.path().join("source.txt");
        std::fs::write(&source, "store file").expect("source");
        let entry = LinkEntry {
            source,
            target: dir.path().join("missing.txt"),
            link_type: LinkType::HardLink,
            dependency_repo: "repo".to_string(),
            source_inode: None,
            source_volume: None,
        };

        let audit = audit_links(&[entry]);

        assert_eq!(audit.conflicts.len(), 1);
        assert_eq!(audit.conflicts[0].reason, LinkConflictReason::TargetMissing);
        assert!(audit.conflicts[0].resolvable);
    }

    #[test]
    fn test_audit_exposes_missing_store_source_as_resolvable_conflict() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let target = dir.path().join("target.txt");
        std::fs::write(&target, "workspace file").expect("target");
        let entry = LinkEntry {
            source: dir.path().join("missing.txt"),
            target,
            link_type: LinkType::HardLink,
            dependency_repo: "repo".to_string(),
            source_inode: None,
            source_volume: None,
        };

        let audit = audit_links(&[entry]);

        assert_eq!(audit.conflicts.len(), 1);
        assert_eq!(audit.conflicts[0].reason, LinkConflictReason::SourceMissing);
        assert!(audit.conflicts[0].resolvable);
    }

    #[test]
    fn test_restore_links_recreates_missing_hardlink() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let source = dir.path().join("source.txt");
        let target = dir.path().join("target.txt");
        std::fs::write(&source, "managed").expect("Failed to write source");
        let (inode, volume) = get_file_inode(&source).expect("Missing file identity");
        let entry = LinkEntry {
            source: source.clone(),
            target: target.clone(),
            link_type: LinkType::HardLink,
            dependency_repo: "repo".to_string(),
            source_inode: Some(inode),
            source_volume: Some(volume),
        };

        restore_links(&[entry]).expect("Failed to restore hardlink");

        assert_eq!(get_file_inode(&source), get_file_inode(&target));
    }

    #[test]
    fn test_cleanup_empty_dirs() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");

        // Create nested empty directories
        let nested = dir.path().join("a/b/c");
        std::fs::create_dir_all(&nested).expect("Failed to create nested dir");

        // Create a file in 'a' to prevent it from being removed
        std::fs::write(dir.path().join("a/keep.txt"), "keep").expect("Failed to write keep file");

        cleanup_empty_dirs(&nested, dir.path()).expect("Failed to cleanup");

        // 'c' and 'b' should be removed (empty), 'a' should stay (has keep.txt)
        assert!(!dir.path().join("a/b/c").exists());
        assert!(!dir.path().join("a/b").exists());
        assert!(dir.path().join("a").exists());
        assert!(dir.path().join("a/keep.txt").exists());
    }

    #[test]
    fn test_cleanup_empty_dirs_stops_at_boundary() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");

        let nested = dir.path().join("a/b");
        std::fs::create_dir_all(&nested).expect("Failed to create nested dir");

        // cleanup with stop_at = nested itself should be a no-op
        cleanup_empty_dirs(&nested, &nested).expect("Failed to cleanup");
        assert!(nested.exists());
    }

    #[test]
    fn test_create_dependency_links_empty_scope() {
        let source_dir = tempfile::tempdir().expect("Failed to create source temp dir");
        let target_dir = tempfile::tempdir().expect("Failed to create target temp dir");

        setup_source_dir(source_dir.path());

        let scope = Scope {
            include: vec![],
            exclude: vec![],
        };

        let entries = create_dependency_links(source_dir.path(), target_dir.path(), &scope, "test")
            .expect("Failed to create dependency links");

        // Empty include means nothing matches
        assert!(entries.is_empty());
    }

    #[test]
    fn test_link_entry_has_correct_type() {
        let source_dir = tempfile::tempdir().expect("Failed to create source temp dir");
        let target_dir = tempfile::tempdir().expect("Failed to create target temp dir");

        setup_source_dir(source_dir.path());

        let scope = Scope {
            include: vec!["src/**/*.rs".to_string()],
            exclude: vec![],
        };

        let entries = create_dependency_links(source_dir.path(), target_dir.path(), &scope, "test")
            .expect("Failed to create dependency links");

        // On macOS/Linux with same partition, should use HardLink
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            for entry in &entries {
                assert_eq!(entry.link_type, LinkType::HardLink);
            }
        }
    }
}
