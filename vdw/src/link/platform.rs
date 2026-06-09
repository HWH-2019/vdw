use cfg_if::cfg_if;
use std::path::Path;

/// Link creation strategy
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkStrategy {
    HardLink,
    Symlink,
    Junction,
}

cfg_if! {
    if #[cfg(target_os = "macos")] {
        pub const PLATFORM: &str = "macos";
        pub const DEFAULT_STRATEGY: LinkStrategy = LinkStrategy::HardLink;
        pub const FALLBACK_STRATEGY: LinkStrategy = LinkStrategy::Symlink;
    } else if #[cfg(target_os = "linux")] {
        pub const PLATFORM: &str = "linux";
        pub const DEFAULT_STRATEGY: LinkStrategy = LinkStrategy::HardLink;
        pub const FALLBACK_STRATEGY: LinkStrategy = LinkStrategy::Symlink;
    } else if #[cfg(target_os = "windows")] {
        pub const PLATFORM: &str = "windows";
        pub const DEFAULT_STRATEGY: LinkStrategy = LinkStrategy::Junction;
        pub const FALLBACK_STRATEGY: LinkStrategy = LinkStrategy::Symlink;
    } else {
        pub const PLATFORM: &str = "unknown";
        pub const DEFAULT_STRATEGY: LinkStrategy = LinkStrategy::Symlink;
        pub const FALLBACK_STRATEGY: LinkStrategy = LinkStrategy::Symlink;
    }
}

/// Check if two paths are on the same filesystem partition
pub fn is_same_partition(path1: &Path, path2: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(m1), Ok(m2)) = (std::fs::metadata(path1), std::fs::metadata(path2)) {
            return m1.dev() == m2.dev();
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (path1, path2);
    }
    true // assume same partition on non-unix
}

/// Determine the link strategy ONCE for an entire dependency (source_dir → target_dir).
/// Cross-partition is the only case that ALWAYS fails for hardlinks, so we check it here.
/// All other hardlink failures (EDR, exotic FS, permissions) are handled by the per-file
/// fallback in create_single_link, which logs the actual error and falls back to symlink.
pub fn determine_strategy(source_dir: &Path, target_dir: &Path) -> LinkStrategy {
    match DEFAULT_STRATEGY {
        LinkStrategy::HardLink => {
            if is_same_partition(source_dir, target_dir) {
                LinkStrategy::HardLink
            } else {
                FALLBACK_STRATEGY
            }
        }
        other => other,
    }
}

/// Select the best link strategy for a specific file.
/// Uses the pre-determined base strategy; overrides based on file/dir and platform.
///
/// Rules:
/// - Directories always use Symlink (hardlinks don't work on dirs)
/// - On Windows, when base strategy is Junction, files must use Symlink
///   (Junction points only support directories, not individual files)
pub fn select_strategy_for_file(source: &Path, base_strategy: LinkStrategy) -> LinkStrategy {
    // Directories always use symlinks (hardlinks don't work on dirs)
    if source.is_dir() {
        return LinkStrategy::Symlink;
    }
    // Windows: Junction only works on directories, not files
    #[cfg(target_os = "windows")]
    if base_strategy == LinkStrategy::Junction {
        return LinkStrategy::Symlink;
    }
    base_strategy
}

/// Try to enable the SeCreateSymbolicLinkPrivilege for the current process.
/// This works when running as Administrator (the privilege exists in the token
/// but is disabled by default). Returns true if the privilege was successfully enabled.
///
/// If the privilege is not available in the token at all (not admin, no Developer Mode),
/// returns false and the caller should fall through to a user-facing error.
#[cfg(windows)]
pub fn try_enable_symlink_privilege() -> bool {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;

    const TOKEN_QUERY: u32 = 0x0008;
    const TOKEN_ADJUST_PRIVILEGES: u32 = 0x0020;
    const SE_PRIVILEGE_ENABLED: u32 = 0x00000002;

    #[repr(C)]
    struct LUID {
        low_part: u32,
        high_part: i32,
    }

    #[repr(C)]
    struct LUID_AND_ATTRIBUTES {
        luid: LUID,
        attributes: u32,
    }

    #[repr(C)]
    struct TOKEN_PRIVILEGES {
        privilege_count: u32,
        privileges: [LUID_AND_ATTRIBUTES; 1],
    }

    extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
        fn OpenProcessToken(
            process_handle: *mut std::ffi::c_void,
            desired_access: u32,
            token_handle: *mut *mut std::ffi::c_void,
        ) -> i32;
        fn LookupPrivilegeValueW(
            system_name: *const u16,
            name: *const u16,
            luid: *mut LUID,
        ) -> i32;
        fn AdjustTokenPrivileges(
            token_handle: *mut std::ffi::c_void,
            disable_all_privileges: i32,
            new_state: *const TOKEN_PRIVILEGES,
            buffer_length: u32,
            previous_state: *mut TOKEN_PRIVILEGES,
            return_length: *mut u32,
        ) -> i32;
        fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
    }

    unsafe {
        let process = GetCurrentProcess();
        let mut token: *mut std::ffi::c_void = ptr::null_mut();

        if OpenProcessToken(process, TOKEN_QUERY | TOKEN_ADJUST_PRIVILEGES, &mut token) == 0 {
            return false;
        }

        // "SeCreateSymbolicLinkPrivilege" as wide string
        let privilege_name: Vec<u16> = OsStr::new("SeCreateSymbolicLinkPrivilege")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let mut luid = LUID {
            low_part: 0,
            high_part: 0,
        };

        if LookupPrivilegeValueW(ptr::null(), privilege_name.as_ptr(), &mut luid) == 0 {
            CloseHandle(token);
            return false;
        }

        let tp = TOKEN_PRIVILEGES {
            privilege_count: 1,
            privileges: [LUID_AND_ATTRIBUTES {
                luid,
                attributes: SE_PRIVILEGE_ENABLED,
            }],
        };

        let result = AdjustTokenPrivileges(
            token,
            0, // don't disable all, just modify specified
            &tp,
            std::mem::size_of::<TOKEN_PRIVILEGES>() as u32,
            ptr::null_mut(),
            ptr::null_mut(),
        );

        CloseHandle(token);

        // AdjustTokenPrivileges returns nonzero even on failure in some cases;
        // must check GetLastError() == ERROR_SUCCESS via its return
        result != 0
    }
}

/// Check if Windows Developer Mode is enabled by reading the registry.
/// Developer Mode grants SeCreateSymbolicLinkPrivilege to all processes,
/// enabling symlink creation without admin rights.
#[cfg(windows)]
pub fn is_developer_mode_enabled() -> bool {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;

    extern "system" {
        fn RegOpenKeyExW(
            key: isize,
            sub_key: *const u16,
            options: u32,
            desired_access: u32,
            result: *mut isize,
        ) -> i32;
        fn RegQueryValueExW(
            key: isize,
            value_name: *const u16,
            reserved: *mut u32,
            type_: *mut u32,
            data: *mut u8,
            data_size: *mut u32,
        ) -> i32;
        fn RegCloseKey(key: isize) -> i32;
    }

    const HKEY_LOCAL_MACHINE: isize = 0x80000002usize as isize;
    const KEY_READ: u32 = 0x20019;
    const REG_DWORD: u32 = 4;
    const ERROR_SUCCESS: i32 = 0;

    unsafe {
        let sub_key: Vec<u16> = std::ffi::OsStr::new(
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock",
        )
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

        let value_name: Vec<u16> = std::ffi::OsStr::new("AllowDevelopmentWithoutDevLicense")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let mut key: isize = 0;
        if RegOpenKeyExW(HKEY_LOCAL_MACHINE, sub_key.as_ptr(), 0, KEY_READ, &mut key)
            != ERROR_SUCCESS
        {
            return false;
        }

        let mut data: u32 = 0;
        let mut data_size: u32 = std::mem::size_of::<u32>() as u32;
        let mut type_: u32 = 0;

        let result = RegQueryValueExW(
            key,
            value_name.as_ptr(),
            ptr::null_mut(),
            &mut type_,
            &mut data as *mut u32 as *mut u8,
            &mut data_size,
        );

        RegCloseKey(key);

        result == ERROR_SUCCESS && type_ == REG_DWORD && data == 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platform_constant() {
        #[cfg(target_os = "macos")]
        assert_eq!(PLATFORM, "macos");
        #[cfg(target_os = "linux")]
        assert_eq!(PLATFORM, "linux");
        #[cfg(target_os = "windows")]
        assert_eq!(PLATFORM, "windows");
    }

    #[test]
    fn test_default_strategy_matches_platform() {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        assert_eq!(DEFAULT_STRATEGY, LinkStrategy::HardLink);
        #[cfg(target_os = "windows")]
        assert_eq!(DEFAULT_STRATEGY, LinkStrategy::Junction);
    }

    #[test]
    fn test_fallback_strategy_is_symlink() {
        assert_eq!(FALLBACK_STRATEGY, LinkStrategy::Symlink);
    }

    #[test]
    fn test_select_strategy_directory_uses_symlink() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let sub_dir = dir.path().join("subdir");
        std::fs::create_dir_all(&sub_dir).expect("Failed to create subdir");

        // Directory sources always get Symlink regardless of base strategy
        let strategy = select_strategy_for_file(&sub_dir, LinkStrategy::HardLink);
        assert_eq!(strategy, LinkStrategy::Symlink);
    }

    #[test]
    fn test_determine_strategy_same_partition_prefers_hardlink() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let source_dir = dir.path().join("source");
        let target_dir = dir.path().join("target");
        std::fs::create_dir_all(&source_dir).expect("Failed to create source dir");
        std::fs::create_dir_all(&target_dir).expect("Failed to create target dir");

        let strategy = determine_strategy(&source_dir, &target_dir);
        // On same partition with hardlink support, should be HardLink
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        assert_eq!(strategy, LinkStrategy::HardLink);
        #[cfg(target_os = "windows")]
        assert_eq!(strategy, LinkStrategy::Junction);
    }

    #[test]
    fn test_select_strategy_for_file_passes_through_base() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "hello").expect("Failed to write test file");

        // For files, select_strategy_for_file returns the base strategy as-is
        assert_eq!(select_strategy_for_file(&file_path, LinkStrategy::HardLink), LinkStrategy::HardLink);
        assert_eq!(select_strategy_for_file(&file_path, LinkStrategy::Symlink), LinkStrategy::Symlink);
    }

    #[test]
    fn test_is_same_partition_same_dir() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        assert!(is_same_partition(dir.path(), dir.path()));
    }

    #[test]
    fn test_link_strategy_equality() {
        assert_eq!(LinkStrategy::HardLink, LinkStrategy::HardLink);
        assert_ne!(LinkStrategy::HardLink, LinkStrategy::Symlink);
        assert_ne!(LinkStrategy::Symlink, LinkStrategy::Junction);
    }
}
