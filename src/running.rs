use std::collections::HashSet;
use sysinfo::System;

/// Lowercase running-process names, snapshotted once per scan.
///
/// Matching a Caches/Logs top-level dir (usually a bundle id like
/// `com.google.Chrome`) against process names is a heuristic: we take the
/// last dot-separated segment ("Chrome") and look for it as a substring of
/// any running process name. Good enough to avoid deleting an active app's
/// cache; not a guarantee for apps with process names unrelated to their
/// bundle id tail.
pub struct RunningApps {
    names: HashSet<String>,
}

impl RunningApps {
    pub fn snapshot() -> Self {
        let sys = System::new_all();
        let names = sys.processes().values().map(|p| p.name().to_string_lossy().to_lowercase()).collect();
        Self { names }
    }

    pub fn matches(&self, dir_name: &str) -> bool {
        let token = dir_name.rsplit('.').next().unwrap_or(dir_name).to_lowercase();
        if token.len() < 3 {
            return false;
        }
        self.names.iter().any(|n| n.contains(&token))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_by_bundle_id_tail() {
        let apps = RunningApps { names: HashSet::from(["google chrome helper".to_string()]) };
        assert!(apps.matches("com.google.Chrome"));
        assert!(!apps.matches("com.example.NotRunning"));
    }

    #[test]
    fn ignores_short_tokens_to_avoid_false_positives() {
        let apps = RunningApps { names: HashSet::from(["finder".to_string()]) };
        assert!(!apps.matches("com.apple.tv")); // "tv" too short a token
    }
}
