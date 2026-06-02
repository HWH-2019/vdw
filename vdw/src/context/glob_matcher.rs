use glob::{MatchOptions, Pattern};
use std::path::Path;

/// Default match options for glob patterns.
/// require_literal_separator: false allows `*` to match `/` when needed,
/// and ensures `**` works correctly for recursive directory matching.
const MATCH_OPTIONS: MatchOptions = MatchOptions {
    case_sensitive: true,
    require_literal_separator: false,
    require_literal_leading_dot: false,
};

/// A compiled glob matcher that supports `**` recursive patterns
pub struct GlobMatcher {
    pattern: Pattern,
    raw: String,
}

impl GlobMatcher {
    /// Create a new glob matcher from a pattern string
    pub fn new(pattern: &str) -> anyhow::Result<Self> {
        let normalized = normalize_pattern(pattern);
        let compiled = Pattern::new(&normalized)?;
        Ok(Self {
            pattern: compiled,
            raw: pattern.to_string(),
        })
    }

    /// Check if a path matches this pattern
    pub fn matches(&self, path: &str) -> bool {
        let normalized_path = normalize_path(path);
        self.pattern.matches_with(&normalized_path, MATCH_OPTIONS)
    }

    /// Get the raw pattern string
    pub fn pattern(&self) -> &str {
        &self.raw
    }
}

/// Normalize a glob pattern for consistent matching
fn normalize_pattern(pattern: &str) -> String {
    let p = pattern.replace('\\', "/");
    // glob crate handles ** natively
    p
}

/// Normalize a file path for consistent matching
fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
}

/// Check if a file path matches any of the given include patterns
/// and does not match any exclude patterns.
///
/// Excludes take priority: if a path matches any exclude pattern,
/// it is rejected even if it also matches an include pattern.
pub fn matches_scope(file_path: &str, include: &[String], exclude: &[String]) -> bool {
    let normalized = normalize_path(file_path);

    // Check excludes first — they take priority
    for pattern in exclude {
        if let Ok(matcher) = GlobMatcher::new(pattern) {
            if matcher.matches(&normalized) {
                return false;
            }
        }
    }

    // Check includes
    for pattern in include {
        if let Ok(matcher) = GlobMatcher::new(pattern) {
            if matcher.matches(&normalized) {
                return true;
            }
        }
    }

    false
}

/// Check if a file path matches any of the given include patterns
/// and does not match any exclude patterns, using Path inputs.
pub fn matches_scope_path(file_path: &Path, include: &[String], exclude: &[String]) -> bool {
    let path_str = file_path.to_string_lossy().replace('\\', "/");
    matches_scope(&path_str, include, exclude)
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- GlobMatcher tests ---

    #[test]
    fn test_glob_matcher_double_star_rs() {
        let matcher = GlobMatcher::new("**/*.rs").unwrap();
        assert!(matcher.matches("main.rs"));
        assert!(matcher.matches("src/main.rs"));
        assert!(matcher.matches("src/foo/bar.rs"));
        assert!(matcher.matches("a/b/c/d.rs"));
        assert!(!matcher.matches("src/main.ts"));
    }

    #[test]
    fn test_glob_matcher_src_components_double_star() {
        let matcher = GlobMatcher::new("src/components/**").unwrap();
        assert!(matcher.matches("src/components/Button.tsx"));
        assert!(matcher.matches("src/components/ui/Modal.tsx"));
        assert!(matcher.matches("src/components/nested/deep/file.ts"));
        assert!(!matcher.matches("src/utils/helper.ts"));
        assert!(!matcher.matches("lib/components/Button.tsx"));
    }

    #[test]
    fn test_glob_matcher_star_test_ts() {
        let matcher = GlobMatcher::new("*.test.ts").unwrap();
        assert!(matcher.matches("app.test.ts"));
        assert!(matcher.matches("utils.test.ts"));
        // With require_literal_separator: false, * matches /, so this also matches
        // This is consistent behavior for the matching engine
        assert!(matcher.matches("src/app.test.ts"));
    }

    #[test]
    fn test_glob_matcher_star_star_only() {
        let matcher = GlobMatcher::new("**").unwrap();
        assert!(matcher.matches("any"));
        assert!(matcher.matches("path/to/file"));
    }

    #[test]
    fn test_glob_matcher_exact_file() {
        let matcher = GlobMatcher::new("README.md").unwrap();
        assert!(matcher.matches("README.md"));
        assert!(!matcher.matches("docs/README.md"));
    }

    #[test]
    fn test_glob_matcher_nested_pattern() {
        let matcher = GlobMatcher::new("src/**/mod.rs").unwrap();
        assert!(matcher.matches("src/mod.rs"));
        assert!(matcher.matches("src/foo/mod.rs"));
        assert!(matcher.matches("src/a/b/c/mod.rs"));
        assert!(!matcher.matches("src/main.rs"));
    }

    #[test]
    fn test_glob_matcher_invalid_pattern() {
        assert!(GlobMatcher::new("[invalid").is_err());
    }

    #[test]
    fn test_glob_matcher_pattern_accessor() {
        let matcher = GlobMatcher::new("**/*.rs").unwrap();
        assert_eq!(matcher.pattern(), "**/*.rs");
    }

    #[test]
    fn test_glob_matcher_backslash_normalization() {
        let matcher = GlobMatcher::new("**/*.rs").unwrap();
        assert!(matcher.matches("src\\main.rs"));
    }

    // --- matches_scope tests ---

    #[test]
    fn test_matches_scope_include_only() {
        let include = vec!["**/*.rs".to_string()];
        let exclude: Vec<String> = vec![];
        assert!(matches_scope("src/main.rs", &include, &exclude));
        assert!(matches_scope("lib/foo.rs", &include, &exclude));
        assert!(!matches_scope("src/main.ts", &include, &exclude));
    }

    #[test]
    fn test_matches_scope_exclude_takes_priority() {
        let include = vec!["**/*.rs".to_string()];
        let exclude = vec!["**/test_*.rs".to_string()];
        assert!(matches_scope("src/main.rs", &include, &exclude));
        assert!(!matches_scope("src/test_foo.rs", &include, &exclude));
        assert!(!matches_scope("test_bar.rs", &include, &exclude));
    }

    #[test]
    fn test_matches_scope_multiple_includes() {
        let include = vec!["**/*.rs".to_string(), "**/*.ts".to_string()];
        let exclude: Vec<String> = vec![];
        assert!(matches_scope("src/main.rs", &include, &exclude));
        assert!(matches_scope("src/app.ts", &include, &exclude));
        assert!(!matches_scope("src/README.md", &include, &exclude));
    }

    #[test]
    fn test_matches_scope_multiple_excludes() {
        let include = vec!["**/*".to_string()];
        let exclude = vec!["**/*.test.ts".to_string(), "**/*.spec.ts".to_string()];
        assert!(matches_scope("src/app.ts", &include, &exclude));
        assert!(!matches_scope("src/app.test.ts", &include, &exclude));
        assert!(!matches_scope("src/app.spec.ts", &include, &exclude));
    }

    #[test]
    fn test_matches_scope_empty_include() {
        let include: Vec<String> = vec![];
        let exclude: Vec<String> = vec![];
        assert!(!matches_scope("any/file.rs", &include, &exclude));
    }

    #[test]
    fn test_matches_scope_all_excluded() {
        let include = vec!["**/*.rs".to_string()];
        let exclude = vec!["**/*.rs".to_string()];
        assert!(!matches_scope("src/main.rs", &include, &exclude));
    }

    #[test]
    fn test_matches_scope_test_ts_exclusion() {
        let include = vec!["**/*.ts".to_string()];
        let exclude = vec!["**/*.test.ts".to_string()];
        assert!(matches_scope("src/utils.ts", &include, &exclude));
        assert!(!matches_scope("src/utils.test.ts", &include, &exclude));
    }

    #[test]
    fn test_matches_scope_path_version() {
        let include = vec!["**/*.rs".to_string()];
        let exclude: Vec<String> = vec![];
        let path = Path::new("src/main.rs");
        assert!(matches_scope_path(path, &include, &exclude));
    }

    // --- normalize_pattern tests ---

    #[test]
    fn test_normalize_pattern_backslash() {
        assert_eq!(normalize_pattern("src\\**\\*.rs"), "src/**/*.rs");
    }

    #[test]
    fn test_normalize_pattern_forward_slash() {
        assert_eq!(normalize_pattern("src/**/*.rs"), "src/**/*.rs");
    }
}
