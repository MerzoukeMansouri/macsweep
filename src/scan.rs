use rayon::prelude::*;
use std::path::{Path, PathBuf};

use crate::running::RunningApps;

/// Path substrings that are never eligible for deletion, regardless of category.
/// Safety floor independent of category scan logic.
const DENYLIST: &[&str] = &[
    "Mobile Documents",  // iCloud / CloudDocs
    "Keychains",
    "com.apple.bird",    // iCloud sync daemon cache
    "com.apple.cloudd",
    "com.apple.security",
    "com.apple.icloud",
];

fn is_denied(path: &Path) -> bool {
    let s = path.to_string_lossy();
    DENYLIST.iter().any(|d| s.contains(d))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    UserCache,
    Logs,
    Trash,
    XcodeJunk,
    Localization,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Category::UserCache => "User Cache",
            Category::Logs => "Logs",
            Category::Trash => "Trash",
            Category::XcodeJunk => "Xcode Derived Data",
            Category::Localization => "Localization files",
        }
    }

}

/// A single deletable unit within a category: one top-level directory or file.
/// Categories delete at this granularity (not per-file) — cheap and matches
/// the atomic-category selection model (no drill-down in V1).
#[derive(Debug, Clone)]
pub struct Item {
    pub path: PathBuf,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct CategoryEntry {
    pub category: Category,
    pub items: Vec<Item>,
    pub total_size: u64,
    pub skipped_running: usize,
    pub selected: bool,
}

impl CategoryEntry {
    pub fn file_count_label(&self) -> String {
        format!("{} items", self.items.len())
    }
}

fn dir_size(path: &Path) -> u64 {
    walkdir::WalkDir::new(path)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum()
}

/// Localization directories that are always kept even inside an in-scope app bundle.
fn is_kept_locale(dir_name: &str) -> bool {
    matches!(dir_name, "en.lproj" | "Base.lproj")
}

fn scan_top_level_roots(base: &Path, running: &RunningApps) -> (Vec<Item>, usize) {
    let Ok(read) = std::fs::read_dir(base) else {
        return (vec![], 0);
    };
    let candidates: Vec<PathBuf> = read.filter_map(std::result::Result::ok).map(|e| e.path()).filter(|p| !is_denied(p)).collect();

    let mut skipped = 0usize;
    let mut items = Vec::new();
    let results: Vec<(PathBuf, u64, bool)> = candidates
        .par_iter()
        .map(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            let in_use = running.matches(name);
            let size = if in_use { 0 } else { dir_size(p) };
            (p.clone(), size, in_use)
        })
        .collect();

    for (path, size, in_use) in results {
        if in_use {
            skipped += 1;
            continue;
        }
        items.push(Item { path, size });
    }
    (items, skipped)
}

fn scan_user_cache(running: &RunningApps) -> CategoryEntry {
    let base = dirs_home().join("Library/Caches");
    let (items, skipped) = scan_top_level_roots(&base, running);
    build_entry(Category::UserCache, items, skipped)
}

fn scan_logs(running: &RunningApps) -> CategoryEntry {
    let base = dirs_home().join("Library/Logs");
    let (items, skipped) = scan_top_level_roots(&base, running);
    build_entry(Category::Logs, items, skipped)
}

fn scan_trash(_running: &RunningApps) -> CategoryEntry {
    let base = dirs_home().join(".Trash");
    let Ok(read) = std::fs::read_dir(&base) else {
        return build_entry(Category::Trash, vec![], 0);
    };
    let candidates: Vec<PathBuf> = read.filter_map(std::result::Result::ok).map(|e| e.path()).filter(|p| !is_denied(p)).collect();
    let items: Vec<Item> = candidates.par_iter().map(|p| Item { path: p.clone(), size: dir_size(p) }).collect();
    build_entry(Category::Trash, items, 0)
}

fn scan_xcode_junk(_running: &RunningApps) -> CategoryEntry {
    let dev = dirs_home().join("Library/Developer/Xcode");
    let roots = ["DerivedData", "iOS DeviceSupport", "watchOS DeviceSupport", "tvOS DeviceSupport"];
    let candidates: Vec<PathBuf> = roots.iter().map(|r| dev.join(r)).filter(|p| p.exists() && !is_denied(p)).collect();
    let items: Vec<Item> = candidates
        .par_iter()
        .flat_map(|root| {
            std::fs::read_dir(root)
                .into_iter()
                .flatten()
                .filter_map(std::result::Result::ok)
                .map(|e| e.path())
                .collect::<Vec<_>>()
        })
        .map(|p| Item { size: dir_size(&p), path: p })
        .collect();
    build_entry(Category::XcodeJunk, items, 0)
}

fn scan_localization(_running: &RunningApps) -> CategoryEntry {
    let user_apps = dirs_home().join("Applications");
    let app_dirs = ["/Applications", &user_apps.to_string_lossy()];
    let mut apps = Vec::new();
    for dir in app_dirs {
        if let Ok(read) = std::fs::read_dir(dir) {
            apps.extend(
                read.filter_map(std::result::Result::ok)
                    .map(|e| e.path())
                    .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("app")),
            );
        }
    }

    let items: Vec<Item> = apps
        .par_iter()
        .flat_map(|app| {
            let resources = app.join("Contents/Resources");
            std::fs::read_dir(&resources)
                .into_iter()
                .flatten()
                .filter_map(std::result::Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("lproj"))
                .filter(|p| {
                    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                    !is_kept_locale(name)
                })
                .collect::<Vec<_>>()
        })
        .filter(|p| !is_denied(p))
        .map(|p| Item { size: dir_size(&p), path: p })
        .collect();
    build_entry(Category::Localization, items, 0)
}

fn build_entry(category: Category, items: Vec<Item>, skipped_running: usize) -> CategoryEntry {
    let total_size = items.iter().map(|i| i.size).sum();
    CategoryEntry { category, items, total_size, skipped_running, selected: true }
}

fn dirs_home() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

pub fn scan_all() -> Vec<CategoryEntry> {
    let running = RunningApps::snapshot();
    let scanners: [fn(&RunningApps) -> CategoryEntry; 5] =
        [scan_user_cache, scan_logs, scan_trash, scan_xcode_junk, scan_localization];
    scanners.par_iter().map(|f| f(&running)).collect()
}

/// Used by delete: guards against ever removing a denylisted path even if it
/// somehow ended up in a category's item list.
pub fn assert_deletable(items: &[Item]) -> Vec<&Item> {
    items.iter().filter(|i| !is_denied(&i.path)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn denylist_blocks_icloud_paths() {
        assert!(is_denied(Path::new("/Users/x/Library/Mobile Documents/com~apple~CloudDocs/foo")));
        assert!(is_denied(Path::new("/Users/x/Library/Keychains/login.keychain")));
        assert!(!is_denied(Path::new("/Users/x/Library/Caches/com.example.App")));
    }

    #[test]
    fn kept_locales_are_never_stripped() {
        assert!(is_kept_locale("en.lproj"));
        assert!(is_kept_locale("Base.lproj"));
        assert!(!is_kept_locale("fr.lproj"));
    }

    #[test]
    fn build_entry_sums_sizes() {
        let items = vec![
            Item { path: PathBuf::from("/a"), size: 100 },
            Item { path: PathBuf::from("/b"), size: 250 },
        ];
        let entry = build_entry(Category::UserCache, items, 2);
        assert_eq!(entry.total_size, 350);
        assert_eq!(entry.skipped_running, 2);
        assert!(entry.selected);
    }

    #[test]
    fn assert_deletable_filters_denied_even_if_leaked_into_items() {
        let items = vec![
            Item { path: PathBuf::from("/Users/x/Library/Keychains/login.keychain"), size: 10 },
            Item { path: PathBuf::from("/Users/x/Library/Caches/com.example.App"), size: 20 },
        ];
        let safe = assert_deletable(&items);
        assert_eq!(safe.len(), 1);
        assert_eq!(safe[0].path, PathBuf::from("/Users/x/Library/Caches/com.example.App"));
    }
}
