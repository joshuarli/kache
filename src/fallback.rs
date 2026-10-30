//! macOS compiler admission uses the peer's kernel audit identity.

pub use kache_protocol::fallback::Attempt;

#[cfg(any(test, target_os = "macos"))]
#[path = "fallback/sandbox.rs"]
mod sandbox;

#[cfg(target_os = "macos")]
#[path = "fallback/macos.rs"]
mod macos;

#[cfg(target_os = "macos")]
pub(crate) fn compiler_peer_may_delegate(
    token: kache_ipc::PeerAuditToken,
    trusted_pipeline: bool,
) -> bool {
    macos::compiler_peer_may_delegate(token, trusted_pipeline)
}
