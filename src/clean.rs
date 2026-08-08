use std::sync::mpsc::{self, Receiver};
use std::thread;

use crate::scan::{CategoryEntry, assert_deletable};

pub enum Progress {
    Item { path: String, done_bytes: u64, total_bytes: u64 },
    Done { per_category: Vec<(&'static str, u64)>, total_freed: u64 },
}

/// Deletes items from the selected categories on a background thread,
/// streaming per-item progress back over the returned channel. Deletion is
/// permanent (no Trash staging) — the caller is expected to have confirmed.
pub fn spawn_delete(entries: Vec<CategoryEntry>, dry_run: bool) -> Receiver<Progress> {
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let selected: Vec<&CategoryEntry> = entries.iter().filter(|e| e.selected).collect();
        let total_bytes: u64 = selected.iter().map(|e| e.total_size).sum();
        let mut done_bytes = 0u64;
        let mut per_category = Vec::new();

        for entry in &selected {
            let mut freed_in_category = 0u64;

            if entry.category.deletes_via_command() {
                let ok = dry_run || std::process::Command::new("xcrun").args(["simctl", "delete", "all"]).status().is_ok_and(|s| s.success());
                if ok {
                    done_bytes += entry.total_size;
                    freed_in_category = entry.total_size;
                    let _ = tx.send(Progress::Item {
                        path: format!("{} (via xcrun simctl)", entry.category.label()),
                        done_bytes,
                        total_bytes,
                    });
                }
                per_category.push((entry.category.label(), freed_in_category));
                continue;
            }

            for item in assert_deletable(&entry.items) {
                if !dry_run {
                    let result = if item.path.is_dir() {
                        std::fs::remove_dir_all(&item.path)
                    } else {
                        std::fs::remove_file(&item.path)
                    };
                    if result.is_err() {
                        // permission denied (e.g. root-owned app bundle) or already gone — skip and move on
                        continue;
                    }
                }
                done_bytes += item.size;
                freed_in_category += item.size;
                let _ = tx.send(Progress::Item { path: item.path.display().to_string(), done_bytes, total_bytes });
            }
            per_category.push((entry.category.label(), freed_in_category));
        }

        let _ = tx.send(Progress::Done { per_category, total_freed: done_bytes });
    });

    rx
}
