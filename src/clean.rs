use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use crate::scan::{assert_deletable, CategoryEntry, Cleanup};

fn root_disk_available() -> u64 {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    disks
        .iter()
        .find(|d| d.mount_point() == Path::new("/"))
        .map_or(0, sysinfo::Disk::available_space)
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

/// Deletes plain filesystem items one at a time via `remove_dir_all`/`remove_file`.
fn clean_filesystem(
    entry: &CategoryEntry,
    dry_run: bool,
    tx: &Sender<Progress>,
    done_bytes: &mut u64,
    total_bytes: u64,
) -> u64 {
    let mut freed = 0u64;
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
        *done_bytes += item.size;
        freed += item.size;
        let _ = tx.send(Progress::Item {
            path: item.path.display().to_string(),
            done_bytes: *done_bytes,
            total_bytes,
        });
    }
    freed
}

/// Wipes all `CoreSimulator` device data in one shot via `xcrun simctl delete all`.
fn clean_simctl(
    entry: &CategoryEntry,
    dry_run: bool,
    tx: &Sender<Progress>,
    done_bytes: &mut u64,
    total_bytes: u64,
) -> (u64, Option<String>) {
    let outcome = if dry_run {
        Ok(())
    } else {
        match std::process::Command::new("xcrun")
            .args(["simctl", "delete", "all"])
            .output()
        {
            Ok(out) if out.status.success() => Ok(()),
            Ok(out) => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
            Err(e) => Err(e.to_string()),
        }
    };
    match outcome {
        Ok(()) => {
            *done_bytes += entry.total_size;
            let _ = tx.send(Progress::Item {
                path: format!("{} (via xcrun simctl)", entry.category.label()),
                done_bytes: *done_bytes,
                total_bytes,
            });
            (entry.total_size, None)
        }
        Err(err) => (0, Some(err)),
    }
}

/// Deletes each stale snapshot's bare timestamp via `tmutil deletelocalsnapshots`.
/// Freed bytes are a measured before/after disk free-space delta, not summed —
/// APFS snapshots are copy-on-write and don't have a meaningful per-item size.
fn clean_tm_snapshots(
    entry: &CategoryEntry,
    dry_run: bool,
    tx: &Sender<Progress>,
    done_bytes: &mut u64,
    total_bytes: u64,
) -> (u64, Option<String>) {
    let before = if dry_run { 0 } else { root_disk_available() };
    let mut failures = Vec::new();
    for item in &entry.items {
        let tag = item.path.to_string_lossy().into_owned();
        if !dry_run {
            match std::process::Command::new("tmutil")
                .args(["deletelocalsnapshots", &tag])
                .output()
            {
                Ok(out) if out.status.success() => {}
                Ok(out) => failures.push(String::from_utf8_lossy(&out.stderr).trim().to_string()),
                Err(e) => failures.push(e.to_string()),
            }
        }
        let _ = tx.send(Progress::Item {
            path: format!("Time Machine snapshot {tag}"),
            done_bytes: *done_bytes,
            total_bytes,
        });
    }
    let freed = if dry_run {
        0
    } else {
        root_disk_available().saturating_sub(before)
    };
    *done_bytes += freed;
    let error = (!failures.is_empty()).then(|| failures.join("; "));
    (freed, error)
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
            let (freed, error) = match entry.category.cleanup() {
                Cleanup::Filesystem => (
                    clean_filesystem(entry, dry_run, &tx, &mut done_bytes, total_bytes),
                    None,
                ),
                Cleanup::SimctlDeleteAll => clean_simctl(entry, dry_run, &tx, &mut done_bytes, total_bytes),
                Cleanup::TmSnapshotThin => clean_tm_snapshots(entry, dry_run, &tx, &mut done_bytes, total_bytes),
            };
            if let Some(err) = error {
                errors.push(format!("{}: {err}", entry.category.label()));
            }
            per_category.push((entry.category.label(), freed));
        }

        let _ = tx.send(Progress::Done {
            per_category,
            total_freed: done_bytes,
            errors,
        });
    });

    rx
}
