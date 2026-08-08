use std::process::Command;

/// Flushes the DNS resolver cache. `dscacheutil -flushcache` alone often
/// isn't enough on modern macOS — the resolver cache lives in mDNSResponder,
/// so it needs a HUP to actually pick up the flush.
pub fn flush_dns() -> anyhow::Result<()> {
    let flush = Command::new("sudo").args(["dscacheutil", "-flushcache"]).status()?;
    anyhow::ensure!(flush.success(), "dscacheutil -flushcache exited with {flush}");

    let restart = Command::new("sudo")
        .args(["killall", "-HUP", "mDNSResponder"])
        .status()?;
    anyhow::ensure!(restart.success(), "killall -HUP mDNSResponder exited with {restart}");

    Ok(())
}

pub struct Action {
    pub label: &'static str,
    pub run: fn() -> anyhow::Result<()>,
}

pub const ACTIONS: &[Action] = &[Action {
    label: "Flush DNS Cache",
    run: flush_dns,
}];
