use std::process::Command;
use sysinfo::System;

/// sysinfo doesn't expose macOS's wired/compressed breakdown (that's a
/// vm_stat-specific concept); we approximate "pressure" as used/total and use
/// swap usage as a proxy signal for compression, which is close enough for a
/// monitor gauge without shelling out to parse `vm_stat` text.
#[derive(Debug, Clone, Copy, Default)]
#[allow(clippy::struct_field_names)] // "_bytes" suffix is the unit, not stutter
pub struct MemStats {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
    pub swap_used_bytes: u64,
    pub swap_total_bytes: u64,
}

impl MemStats {
    pub fn pressure_pct(&self) -> f64 {
        if self.total_bytes == 0 {
            return 0.0;
        }
        (self.used_bytes as f64 / self.total_bytes as f64) * 100.0
    }

    pub fn swap_pct(&self) -> f64 {
        if self.swap_total_bytes == 0 {
            return 0.0;
        }
        (self.swap_used_bytes as f64 / self.swap_total_bytes as f64) * 100.0
    }
}

pub fn sample(sys: &mut System) -> MemStats {
    sys.refresh_memory();
    MemStats {
        total_bytes: sys.total_memory(),
        used_bytes: sys.used_memory(),
        available_bytes: sys.available_memory(),
        swap_used_bytes: sys.used_swap(),
        swap_total_bytes: sys.total_swap(),
    }
}

/// Shells out to `sudo purge` — terminal owns the password prompt, we don't
/// handle credentials ourselves. Blocking call; run off the UI thread.
pub fn free_up() -> anyhow::Result<()> {
    let status = Command::new("sudo").arg("purge").status()?;
    anyhow::ensure!(status.success(), "purge exited with {status}");
    Ok(())
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact values are deterministic here, no rounding involved
mod tests {
    use super::*;

    #[test]
    fn pressure_pct_handles_zero_total() {
        let stats = MemStats::default();
        assert_eq!(stats.pressure_pct(), 0.0);
        assert_eq!(stats.swap_pct(), 0.0);
    }

    #[test]
    fn pressure_pct_computes_ratio() {
        let stats = MemStats {
            total_bytes: 100,
            used_bytes: 40,
            ..Default::default()
        };
        assert_eq!(stats.pressure_pct(), 40.0);
    }
}
