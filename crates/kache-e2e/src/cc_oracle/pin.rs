//! The pinned ccache binary for this host's lane, read from `lock.toml`.

use anyhow::{Context, Result};
use deser::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct CcachePin {
    pub lane: String,
    pub url: String,
    pub sha256: String,
}

#[derive(Deserialize)]
struct Lock {
    reference: Reference,
}

#[derive(Deserialize)]
struct Reference {
    ccache_binaries: Vec<CcachePin>,
}

/// The campaign lane a host belongs to, from the platform this binary was
/// built for. Intel macOS is not a campaign lane.
pub fn lane(os: &str, arch: &str, musl: bool) -> Option<&'static str> {
    match (os, arch, musl) {
        ("macos", "aarch64", _) => Some("M1"),
        ("linux", "x86_64", false) => Some("L1"),
        ("linux", "aarch64", false) => Some("L2"),
        ("linux", "x86_64", true) => Some("L3"),
        ("linux", "aarch64", true) => Some("L4"),
        _ => None,
    }
}

pub fn pin_for(lock_text: &str, lane: &str) -> Result<CcachePin> {
    let lock: Lock = deser_toml::from_str(lock_text).context("parsing lock.toml")?;
    lock.reference
        .ccache_binaries
        .into_iter()
        .find(|p| p.lane == lane)
        .with_context(|| format!("lock.toml pins no ccache binary for lane {lane}"))
}

pub fn host_ccache_pin(lock: &Path) -> Result<CcachePin> {
    let lane = lane(std::env::consts::OS, std::env::consts::ARCH, cfg!(target_env = "musl"))
        .with_context(|| format!("{}-{} is not a campaign lane", std::env::consts::OS, std::env::consts::ARCH))?;
    let text = std::fs::read_to_string(lock).with_context(|| format!("reading {}", lock.display()))?;
    pin_for(&text, lane)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lanes_follow_os_arch_and_libc() {
        assert_eq!(lane("macos", "aarch64", false), Some("M1"));
        assert_eq!(lane("linux", "x86_64", false), Some("L1"));
        assert_eq!(lane("linux", "aarch64", false), Some("L2"));
        assert_eq!(lane("linux", "x86_64", true), Some("L3"));
        assert_eq!(lane("linux", "aarch64", true), Some("L4"));
        assert_eq!(lane("macos", "x86_64", false), None);
    }

    #[test]
    fn the_committed_lock_pins_every_lane() {
        let text = include_str!("../../../../campaigns/cpp/lock.toml");
        for lane in ["M1", "L1", "L2", "L3", "L4"] {
            let pin = pin_for(text, lane).unwrap();
            assert!(pin.url.contains("ccache-4.14.3"), "{lane}: {}", pin.url);
            assert_eq!(pin.sha256.len(), 64, "{lane}");
        }
        assert!(pin_for(text, "L9").is_err());
    }
}
