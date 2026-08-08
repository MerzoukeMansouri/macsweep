use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use crate::scan::{assert_deletable, CategoryEntry, Cleanup};

fn root_disk_available() -> u64 {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    disks.iter().find(|d| d.mount_point() == Path::new("/")).map_or(0, sysinfo::Disk::available_space)
}

pub enum Progress {
    Item {
        path: String,
        done_bytes: u64,
        total_bytes: u64,
    },
    Done {
        per_category: Vec<(&'static str, u64)>,
        total_freed: u64,
        errors: Vec<String>,
    },
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
        let mut errors = Vec::new();

        for entry in &selected {
            let mut freed_in_category = 0u64;

            match entry.category.cleanup() {
                Cleanup::SimctlDeleteAll => {
                    let outcome = if dry_run {
                        Ok(())
                    } else {
                        match std::process::Command::new("xcrun").args(["simctl", "delete", "all"]).output() {
                            Ok(out) if out.status.success() => Ok(()),
                            Ok(out) => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
                            Err(e) => Err(e.to_string()),
                        }
                    };
                    match outcome {
                        Ok(()) => {
                            done_bytes += entry.total_size;
                            freed_in_category = entry.total_size;
                            let _ = tx.send(Progress::Item {
                                path: format!("{} (via xcrun simctl)", entry.category.label()),
                                done_bytes,
                                total_bytes,
                            });
                        }
                        Err(err) => errors.push(format!("{}: {err}", entry.category.label())),
                    }
                    per_category.push((entry.category.label(), freed_in_category));
                    continue;
                }
                Cleanup::TmSnapshotThin => {
                    let before = if dry_run { 0 } else { root_disk_available() };
                    let mut failures = Vec::new();
                    for item in &entry.items {
                        let tag = item.path.to_string_lossy().into_owned();
                        if !dry_run {
                            match std::process::Command::new("tmutil").args(["deletelocalsnapshots", &tag]).output() {
                                Ok(out) if out.status.success() => {}
                                Ok(out) => failures.push(String::from_utf8_lossy(&out.stderr).trim().to_string()),
                                Err(e) => failures.push(e.to_string()),
                            }
                        }
                        let _ = tx.send(Progress::Item {
                            path: format!("Time Machine snapshot {tag}"),
                            done_bytes,
                            total_bytes,
                        });
                    }
                    let freed = if dry_run { 0 } else { root_disk_available().saturating_sub(before) };
                    done_bytes += freed;
                    freed_in_category = freed;
                    if !failures.is_empty() {
                        errors.push(format!("{}: {}", entry.category.label(), failures.join("; ")));
                    }
                    per_category.push((entry.category.label(), freed_in_category));
                    continue;
                }
                Cleanup::Filesystem => {}
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
                let _ = tx.send(Progress::Item {
                    path: item.path.display().to_string(),
                    done_bytes,
                    total_bytes,
                });
            }
            per_category.push((entry.category.label(), freed_in_category));
        }

        let _ = tx.send(Progress::Done {
            per_category,
            total_freed: done_bytes,
            errors,
        });
    });

    rx
}
