use anyhow::Result;

use crate::config::parser;
use crate::context::{file_list, filter};

/// Preview all files in the AI context
pub fn preview() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let config = parser::load_config(&cwd)?;

    println!("AI Context Preview");
    println!("==================\n");

    for dep in &config.dependencies {
        let dep_path = cwd.join(&dep.path);
        println!("📦 {} ({})", dep.repo, dep.path);

        if !dep_path.exists() {
            println!("   ⚠️  Path not linked yet. Run 'vdw link' first.\n");
            continue;
        }

        let files = filter::filter_files(&dep_path, &dep.scope)?;

        if files.is_empty() {
            println!("   (no files match scope)\n");
        } else {
            for file in &files {
                println!("   {}", file.display());
            }
            println!("   --- {} file(s)\n", files.len());
        }
    }

    Ok(())
}

/// Show context statistics
pub fn stats() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let config = parser::load_config(&cwd)?;

    let summary = file_list::generate_context_summary(&config, &cwd)?;

    println!("AI Context Statistics");
    println!("=====================\n");
    println!("Total files in context: {}", summary.total_files);
    println!("Total size: {}\n", format_size(summary.total_size_bytes));

    println!("{:<40} {:>8} {:>10}", "DEPENDENCY", "FILES", "SIZE");
    println!("{}", "-".repeat(60));

    for dep in &summary.per_dependency {
        println!(
            "{:<40} {:>8} {:>10}",
            truncate(&dep.path, 38),
            dep.file_count,
            format_size(dep.size_bytes),
        );
    }

    Ok(())
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("...{}", &s[s.len() - max_len + 3..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_size_bytes() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
    }

    #[test]
    fn test_format_size_kilobytes() {
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
    }

    #[test]
    fn test_format_size_megabytes() {
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
        assert_eq!(format_size(1024 * 1024 * 2 + 512 * 1024), "2.5 MB");
    }

    #[test]
    fn test_truncate_short() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_exact() {
        assert_eq!(truncate("12345", 5), "12345");
    }

    #[test]
    fn test_truncate_long() {
        let result = truncate("a_very_long_path_string", 10);
        assert_eq!(result, "..._string");
        assert_eq!(result.len(), 10);
    }
}
