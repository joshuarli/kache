//! macOS sandbox inspection for compiler handoff.
// The Linux mutation lane cannot execute Darwin process APIs. The policy
// decisions live in sandbox.rs and run under Linux mutations.
use super::sandbox;

// This is private macOS SPI. Resolve it at runtime so an unavailable probe
// never prevents compilation or startup of Kache itself.
type SandboxCheck =
    unsafe extern "C" fn(libc::pid_t, *const libc::c_char, libc::c_int, ...) -> libc::c_int;

/// Native children stay in the client. Pipeline reads still use daemon authority,
/// so sandboxed peers require explicit trust and unknown policy stays local.
pub(super) fn compiler_peer_may_delegate(
    token: kache_ipc::PeerAuditToken,
    trusted_pipeline: bool,
) -> bool {
    type CheckToken = unsafe extern "C" fn(
        kache_ipc::PeerAuditToken,
        *const libc::c_char,
        libc::c_int,
        ...
    ) -> libc::c_int;
    let Some(policy) = Policy::load() else {
        return false;
    };
    let check = unsafe { libc::dlsym(policy.library, c"sandbox_check_by_audit_token".as_ptr()) };
    if check.is_null() {
        return false;
    }
    let check: CheckToken = unsafe { std::mem::transmute(check) };
    unsafe {
        let peer = check(token, std::ptr::null(), 0);
        let daemon = (policy.check)(libc::getpid(), std::ptr::null(), 0);
        sandbox::permits_pipeline(peer, daemon, trusted_pipeline)
    }
}
struct Policy {
    library: *mut libc::c_void,
    check: SandboxCheck,
}
impl Policy {
    fn load() -> Option<Self> {
        unsafe {
            let library = libc::dlopen(c"/usr/lib/libsandbox.dylib".as_ptr(), libc::RTLD_LAZY);
            if library.is_null() {
                return None;
            }
            let check = libc::dlsym(library, c"sandbox_check".as_ptr());
            if check.is_null() {
                libc::dlclose(library);
                return None;
            }
            Some(Self {
                library,
                check: std::mem::transmute::<*mut libc::c_void, SandboxCheck>(check),
            })
        }
    }
}
impl Drop for Policy {
    fn drop(&mut self) {
        unsafe {
            libc::dlclose(self.library);
        }
    }
}
