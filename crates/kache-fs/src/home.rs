//! Home and cache directories for the current user.
//!
//! Two functions, the only `dirs` queries kache ever made: [`home_dir`]
//! and [`cache_dir`], with the same per-platform answers. Unix reads
//! `$HOME` with a passwd fallback when it is unset; Linux honors
//! `$XDG_CACHE_HOME` when absolute; macOS uses `~/Library/Caches`;
//! Windows asks the shell for the profile and local-app-data folders.

use std::path::PathBuf;

/// The current user's home directory: `$HOME` on unix (falling back to
/// the passwd entry when unset), the profile folder on Windows.
pub fn home_dir() -> Option<PathBuf> {
    imp::home_dir()
}

/// Directory for cached data: `$XDG_CACHE_HOME` when absolute, else
/// `~/.cache` on Linux; `~/Library/Caches` on macOS; local app data on
/// Windows.
pub fn cache_dir() -> Option<PathBuf> {
    imp::cache_dir()
}

#[cfg(unix)]
mod imp {
    use std::ffi::{CStr, OsString};
    use std::mem;
    use std::os::unix::ffi::OsStringExt;
    use std::path::PathBuf;
    use std::ptr;

    pub fn home_dir() -> Option<PathBuf> {
        std::env::var_os("HOME")
            .filter(|home| !home.is_empty())
            .or_else(passwd_home)
            .map(PathBuf::from)
    }

    pub fn cache_dir() -> Option<PathBuf> {
        #[cfg(target_os = "macos")]
        {
            home_dir().map(|home| home.join("Library/Caches"))
        }
        #[cfg(not(target_os = "macos"))]
        {
            std::env::var_os("XDG_CACHE_HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .or_else(|| home_dir().map(|home| home.join(".cache")))
        }
    }

    fn passwd_home() -> Option<OsString> {
        // Mirrors the libc fallback the previous implementation used: look
        // up the current uid when `$HOME` is missing (daemons, launchd).
        // Safety: `storage` has `capacity` allocated bytes, so the buffer
        // pointer is valid for `getpwuid_r` to fill; the entry's `pw_dir`
        // points into that buffer and is copied out before it drops.
        let capacity = unsafe {
            match libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) {
                undersized if undersized < 0 => 512usize,
                sized => sized as usize,
            }
        };
        let mut storage = Vec::with_capacity(capacity);
        let mut entry: libc::passwd = unsafe { mem::zeroed() };
        let mut found = ptr::null_mut();
        let status = unsafe {
            libc::getpwuid_r(
                libc::getuid(),
                &mut entry,
                storage.as_mut_ptr() as *mut libc::c_char,
                capacity,
                &mut found,
            )
        };
        if status != 0 || found.is_null() || entry.pw_dir.is_null() {
            return None;
        }
        let dir = unsafe { CStr::from_ptr(entry.pw_dir) };
        if dir.is_empty() {
            return None;
        }
        Some(OsStringExt::from_vec(dir.to_bytes().to_vec()))
    }
}

#[cfg(windows)]
mod imp {
    use std::ffi::{OsString, c_void};
    use std::os::windows::ffi::OsStringExt;
    use std::path::PathBuf;
    use std::ptr;
    use std::slice;

    /// Binary layout of a Windows `GUID`: the first three fields are
    /// little-endian integers, the last eight bytes are stored as-is.
    #[repr(C)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    impl Guid {
        const fn from_u128(value: u128) -> Self {
            Guid {
                data1: (value >> 96) as u32,
                data2: (value >> 80) as u16,
                data3: (value >> 64) as u16,
                data4: (value as u64).to_be_bytes(),
            }
        }
    }

    const FOLDERID_PROFILE: Guid = Guid::from_u128(0x5e6c858f_0e22_4760_9afe_ea3317b67173);
    const FOLDERID_LOCAL_APP_DATA: Guid =
        Guid::from_u128(0xf1b32785_6fba_4fcf_9d55_7b8e7f157091);

    #[link(name = "shell32")]
    unsafe extern "system" {
        fn SHGetKnownFolderPath(
            rfid: *const Guid,
            flags: u32,
            token: *mut c_void,
            path: *mut *mut u16,
        ) -> i32;
    }

    #[link(name = "ole32")]
    unsafe extern "system" {
        fn CoTaskMemFree(pv: *const c_void);
    }

    fn known_folder(id: &Guid) -> Option<PathBuf> {
        // Safety: SHGetKnownFolderPath writes a NUL-terminated UTF-16
        // buffer allocated with CoTaskMemAlloc through `raw` on success
        // (S_OK == 0) and leaves it null or frees it on failure. The
        // buffer is read once, copied to an owned Vec, then freed, so no
        // aliasing or lifetime escapes the block.
        let wide: Vec<u16> = unsafe {
            let mut raw: *mut u16 = ptr::null_mut();
            let status = SHGetKnownFolderPath(id, 0, ptr::null_mut(), &mut raw);
            if status != 0 || raw.is_null() {
                if !raw.is_null() {
                    CoTaskMemFree(raw as *const c_void);
                }
                return None;
            }
            let mut len = 0usize;
            while *raw.add(len) != 0 {
                len += 1;
            }
            let owned = slice::from_raw_parts(raw, len).to_vec();
            CoTaskMemFree(raw as *const c_void);
            owned
        };
        Some(PathBuf::from(OsString::from_wide(&wide)))
    }

    pub fn home_dir() -> Option<PathBuf> {
        known_folder(&FOLDERID_PROFILE)
    }

    pub fn cache_dir() -> Option<PathBuf> {
        known_folder(&FOLDERID_LOCAL_APP_DATA)
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    use std::path::PathBuf;

    pub fn home_dir() -> Option<PathBuf> {
        None
    }

    pub fn cache_dir() -> Option<PathBuf> {
        None
    }
}
