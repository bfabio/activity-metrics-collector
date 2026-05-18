use std::path::{Path, PathBuf};

/// XDG cache base for persisted vitality data, ending in the app subdir.
/// A relative `XDG_CACHE_HOME` is ignored per the XDG spec.
pub fn cache_root() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir)
        .join("activity-metrics-collector")
}

pub fn split_full_name(name: &str) -> (String, String) {
    match name.rsplit_once('/') {
        Some((vendor, repo)) => (vendor.to_string(), repo.to_string()),
        None => (String::new(), name.to_string()),
    }
}

pub fn cache_path(root: &Path, host: &str, full_name: &str) -> PathBuf {
    let (vendor, repo) = split_full_name(full_name);
    root
        .join("repos")
        .join(host)
        .join(vendor)
        .join(repo)
        .join("vitality.json")
}
