use std::path::Path;

/// Git for Windows accepts drive/UNC paths, but its local transport does not
/// consistently accept the verbatim prefix returned by Rust canonicalization.
pub fn git_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    #[cfg(windows)]
    {
        normalize_windows_git_path(&value)
    }
    #[cfg(not(windows))]
    {
        value.into_owned()
    }
}

#[cfg(any(windows, test))]
fn normalize_windows_git_path(value: &str) -> String {
    if let Some(unc) = value.strip_prefix(r"\\?\UNC\") {
        format!("//{}", unc.replace('\\', "/"))
    } else {
        value
            .strip_prefix(r"\\?\")
            .unwrap_or(value)
            .replace('\\', "/")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_git_arguments_preserve_drive_unc_spaces_and_unicode() {
        for (source, expected) in [
            (r"\\?\C:\Users\开发 者\repo", "C:/Users/开发 者/repo"),
            (r"C:\work\repo", "C:/work/repo"),
            (r"\\?\UNC\server\share\repo", "//server/share/repo"),
            (r"\\server\share\repo", "//server/share/repo"),
            ("C:/work/repo", "C:/work/repo"),
        ] {
            assert_eq!(normalize_windows_git_path(source), expected);
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_git_arguments_keep_literal_backslashes() {
        assert_eq!(git_path(Path::new(r"/work/a\b")), r"/work/a\b");
    }
}
