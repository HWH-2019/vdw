use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Result;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::types::workspace::{LinkEntry, LinkType, WorkspaceState};

/// Debounce window in milliseconds for file system events
const DEBOUNCE_MS: u64 = 200;

/// A tracked HardLink entry with its inode metadata
#[derive(Debug, Clone)]
struct TrackedLink {
    source: PathBuf,
    target: PathBuf,
    source_inode: u64,
    source_volume: u64,
    repo_url: String,
}

/// Rate-limiting state per file
#[derive(Debug)]
struct FileAlertState {
    last_alert: Instant,
}

/// Start the file watcher for broken HardLink detection.
/// Loads state.json, then watches the workspace directory for file changes.
/// Runs until the watcher is disconnected (process termination).
pub fn start_watch(workspace_root: &Path) -> Result<()> {
    let state = WorkspaceState::load(workspace_root)?;

    // Collect all HardLink entries with valid inode metadata
    let tracked_links: Vec<TrackedLink> = state
        .links
        .iter()
        .filter(|e| e.link_type == LinkType::HardLink)
        .filter_map(|e| {
            let source_inode = e.source_inode?;
            let source_volume = e.source_volume?;
            Some(TrackedLink {
                source: e.source.clone(),
                target: e.target.clone(),
                source_inode,
                source_volume,
                repo_url: e.dependency_repo.clone(),
            })
        })
        .collect();

    if tracked_links.is_empty() {
        eprintln!("ℹ️  No HardLink entries with inode metadata found.");
        eprintln!("   Run 'vdw link' first to record inode information.");
        return Ok(());
    }

    // Build a lookup map: target path -> TrackedLink
    let link_map: HashMap<PathBuf, TrackedLink> = tracked_links
        .into_iter()
        .map(|l| (l.target.clone(), l))
        .collect();

    eprintln!(
        "🔍 Watching {} HardLink(s) for broken links... (Ctrl+C to stop)",
        link_map.len()
    );

    // Rate-limiting: track last alert time per file
    let alert_states: parking_lot::Mutex<HashMap<PathBuf, FileAlertState>> =
        parking_lot::Mutex::new(HashMap::new());
    let link_map = link_map;

    // Create the file watcher
    let (tx, rx) = std::sync::mpsc::channel::<Result<Event, notify::Error>>();

    let mut watcher = RecommendedWatcher::new(tx, Config::default())?;
    watcher.watch(workspace_root, RecursiveMode::Recursive)?;

    // Debounce: track last event time per file
    let mut debounce_map: HashMap<PathBuf, Instant> = HashMap::new();

    // Event loop — blocks until watcher is disconnected (process termination)
    loop {
        match rx.recv() {
            Ok(Ok(event)) => {
                process_event(
                    &event,
                    &link_map,
                    &alert_states,
                    &mut debounce_map,
                    workspace_root,
                );
            }
            Ok(Err(e)) => {
                eprintln!("⚠️  Watch error: {}", e);
            }
            Err(_) => {
                break;
            }
        }
    }

    eprintln!("\n Watch stopped.");
    Ok(())
}

/// Process a single file system event
fn process_event(
    event: &Event,
    link_map: &HashMap<PathBuf, TrackedLink>,
    alert_states: &parking_lot::Mutex<HashMap<PathBuf, FileAlertState>>,
    debounce_map: &mut HashMap<PathBuf, Instant>,
    workspace_root: &Path,
) {
    // We care about Create and Modify events (atomic save creates a new file = Create)
    let is_relevant = matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_));

    if !is_relevant {
        return;
    }

    let now = Instant::now();

    for path in &event.paths {
        // Check if this path is a tracked link target
        if !link_map.contains_key(path) {
            continue;
        }

        // Debounce: skip if we've seen this path within DEBOUNCE_MS
        if let Some(last) = debounce_map.get(path) {
            if now.duration_since(*last).as_millis() < DEBOUNCE_MS as u128 {
                // Update debounce timer
                debounce_map.insert(path.clone(), now);
                return;
            }
        }
        debounce_map.insert(path.clone(), now);

        // Get tracked link info
        let tracked = match link_map.get(path) {
            Some(t) => t.clone(),
            None => continue,
        };

        // Check if the target file still exists
        if !path.exists() {
            // File was deleted, will be detected on next create event
            continue;
        }

        // Get current inode info of the target file
        let current_inode = get_current_inode(path);

        // Compare with expected inode
        let is_broken = match current_inode {
            Some((cur_inode, cur_volume)) => {
                cur_inode != tracked.source_inode || cur_volume != tracked.source_volume
            }
            None => {
                // Can't read inode — assume broken
                true
            }
        };

        if !is_broken {
            return;
        }

        // Rate-limiting: max 1 alert per file per minute
        {
            let mut states = alert_states.lock();
            if let Some(alert_state) = states.get(path) {
                if now.duration_since(alert_state.last_alert).as_secs() < 60 {
                    return; // Skip — already alerted within last minute
                }
            }
            states.insert(path.clone(), FileAlertState { last_alert: now });
        }

        // Check if store source file still exists
        let source_missing = !tracked.source.exists();

        // Print alert
        let rel_path = path
            .strip_prefix(workspace_root)
            .unwrap_or(path)
            .display()
            .to_string()
            .replace('\\', "/");

        eprintln!("{}", "━".repeat(79));
        eprintln!("⚠️  HardLink 断链检测");
        eprintln!();
        eprintln!("   文件: {}", rel_path);
        eprintln!("   原因: 编辑器原子保存导致 inode 变更");

        if source_missing {
            eprintln!();
            eprintln!("   🚫 Store 源文件已丢失，请执行 vdw repo update");
        } else {
            eprintln!();
            eprintln!("   修复: vdw link --repo {}", tracked.repo_url);
        }

        eprintln!();
        eprintln!("   建议: 关闭 VS Code 原子保存以避免反复断链");
        eprintln!("     设置 → \"files.atomicSave\": false");
        eprintln!("{}", "━".repeat(79));
    }
}

/// Get the current (inode, volume) for a file path.
fn get_current_inode(path: &Path) -> Option<(u64, u64)> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let Ok(meta) = std::fs::metadata(path) {
            return Some((meta.ino(), meta.dev()));
        }
        None
    }
    #[cfg(windows)]
    {
        get_file_inode_windows(path)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        None
    }
}

/// Windows-specific: get file index and volume serial number using GetFileInformationByHandle.
#[cfg(windows)]
fn get_file_inode_windows(path: &Path) -> Option<(u64, u64)> {
    use std::fs::File;
    use std::os::windows::io::AsRawHandle;

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
    None
}

/// Run a one-time full scan of all HardLink entries in the workspace.
/// Returns the number of broken links found.
pub fn check_links(workspace_root: &Path) -> Result<usize> {
    let state = WorkspaceState::load(workspace_root)?;

    // Collect HardLink entries with inode metadata
    let tracked: Vec<&LinkEntry> = state
        .links
        .iter()
        .filter(|e| e.link_type == LinkType::HardLink)
        .filter(|e| e.source_inode.is_some() && e.source_volume.is_some())
        .collect();

    if tracked.is_empty() {
        // Check if there are HardLinks without inode info
        let has_hardlinks = state
            .links
            .iter()
            .any(|e| e.link_type == LinkType::HardLink);
        if has_hardlinks {
            eprintln!("🔍 正在检查 HardLink 健康状态...");
            eprintln!("   无法检查: state.json 中缺少 inode 信息");
            eprintln!("   请先执行 `vdw link` 以记录 inode 元数据");
            return Err(anyhow::anyhow!("Missing inode metadata in state.json"));
        } else {
            return Ok(0);
        }
    }

    // Group by repo for nicer output
    let mut repo_count: HashMap<&str, usize> = HashMap::new();
    for entry in &tracked {
        repo_count
            .entry(&entry.dependency_repo)
            .and_modify(|c| *c += 1)
            .or_insert(1);
    }

    eprintln!("🔍 正在检查 HardLink 健康状态...");
    eprintln!(
        "   共 {} 个 HardLink 条目，分布于 {} 个仓库",
        tracked.len(),
        repo_count.len()
    );
    eprintln!();

    let mut broken_count = 0;

    for entry in &tracked {
        let target = &entry.target;
        if !target.exists() {
            // Target doesn't exist — count as broken
            print_broken_link(entry, workspace_root, false);
            broken_count += 1;
            continue;
        }

        // Compare current inode with stored inode
        let current = get_current_inode(target);
        let expected_inode = entry.source_inode.unwrap_or(0);
        let expected_volume = entry.source_volume.unwrap_or(0);

        let is_broken = match current {
            Some((cur_inode, cur_volume)) => {
                cur_inode != expected_inode || cur_volume != expected_volume
            }
            None => true,
        };

        if is_broken {
            let source_missing = !entry.source.exists();
            print_broken_link(entry, workspace_root, source_missing);
            broken_count += 1;
        }
    }

    if broken_count > 0 {
        eprintln!("💡 提示: 使用 `vdw link --watch` 可实时监控断链");
        eprintln!("   建议: 关闭 VS Code 原子保存 \"files.atomicSave\": false");
    }

    Ok(broken_count)
}

/// Print a broken link alert to stderr
fn print_broken_link(entry: &LinkEntry, workspace_root: &Path, source_missing: bool) {
    let rel_path = entry
        .target
        .strip_prefix(workspace_root)
        .unwrap_or(&entry.target)
        .display()
        .to_string()
        .replace('\\', "/");

    eprintln!("⚠️  {}", rel_path);
    eprintln!("     修复: vdw link --repo {}", entry.dependency_repo);

    if source_missing {
        eprintln!("     🚫 Store 源文件已丢失，请执行 vdw repo update");
    }
    eprintln!();
}
