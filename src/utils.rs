use std::path::Path;
use humansize::{format_size, BINARY};

pub fn format_file_size(size: u64) -> String {
    format_size(size, BINARY)
}

pub fn get_file_extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|s| s.to_lowercase())
}

pub fn is_hidden_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.starts_with('.'))
        .unwrap_or(false)
}

pub fn get_relative_path<'a>(path: &'a Path, base: &Path) -> Option<&'a Path> {
    path.strip_prefix(base).ok()
}

pub fn calculate_percentage(part: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        (part as f64 / total as f64) * 100.0
    }
}
