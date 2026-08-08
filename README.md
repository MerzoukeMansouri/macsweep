# macsweep

CleanMyMac-style TUI for cleaning junk files and freeing RAM on macOS.

![Rust](https://img.shields.io/badge/rust-stable-orange?logo=rust&logoColor=white)
![License](https://img.shields.io/badge/license-MIT-blue)

## Features

- **Junk Cleanup**: user/app cache, logs, trash, Xcode derived data & device support, unused app localizations, stale iOS device backups, Xcode Simulators, stale Time Machine local snapshots
- **Memory**: live used/swap/available gauges, `sudo purge` free-up
- **Maintenance**: flush DNS cache (`sudo dscacheutil -flushcache` + `sudo killall -HUP mDNSResponder`)
- Runs entirely local, permanent delete (no undo) — always review the checklist and confirm before cleaning
- `--dry-run` flag scans and reports without deleting anything

## Install

```bash
brew install merzoukemansouri/homebrew/macsweep
```

or tap first, then install:

```bash
brew tap merzoukemansouri/homebrew
brew install macsweep
```

or from source:

```bash
cargo install --path .
```

## Usage

```bash
macsweep            # normal mode
macsweep --dry-run  # scan and report only, never deletes
```

| Key | Action |
|---|---|
| `Tab` | switch focus between sidebar and panel |
| `↑`/`↓` | navigate |
| `space` | toggle category selection (Junk panel) |
| `c` | clean selected categories (Junk panel) |
| `p` | free up RAM via `sudo purge` (Memory panel) |
| `Enter` | run selected action (Maintenance panel) |
| `y`/`N` | confirm/cancel a pending delete |
| `q` | quit |

## Safety

- Hardcoded denylist (iCloud, Keychains, security daemons) is never deletable regardless of category
- Caches belonging to a currently-running app are skipped automatically
- Categories are atomic (whole-category select, no per-file drill-down) in V1
- Deletion is permanent — there is no Trash staging
- iOS Device Backups: the most recently modified backup is protected automatically and never listed as deletable — only stale/older device backups are eligible
- Xcode Simulators: cleaned via `xcrun simctl delete all` (device data only, not installed runtime images)
- Time Machine Snapshots: the most recent local snapshot is protected automatically, same as iOS backups — only stale ones are eligible. Freed size is a measured disk free-space delta (before/after `tmutil deletelocalsnapshots`), not a summed estimate, since APFS snapshots are copy-on-write and don't have a meaningful per-snapshot size

## What's excluded from V1

- `/tmp` — macOS auto-purges it on reboot; low value, some risk
- Custom exclude config file — hardcoded categories/denylist only for now
