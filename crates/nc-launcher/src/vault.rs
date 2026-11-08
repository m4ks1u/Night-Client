//! Storage for long-lived secrets (Microsoft refresh tokens).
//!
//! On Windows, secrets are encrypted with DPAPI (`CryptProtectData`, current
//! user scope, UI forbidden) plus application-specific entropy, and written
//! atomically to `<dir>/<key>.ncv`. Only the same Windows user on the same
//! machine can decrypt them. Nothing secret ever goes to SQLite or logs.
//!
//! Windows Credential Manager was not used: its blobs are limited to about
//! 2.5 KB, which Microsoft refresh tokens can approach.

use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    sync::Mutex,
};

use nc_core::Secret;

use crate::fsutil;

pub trait TokenVault: Send + Sync {
    fn put(&self, key: &str, secret: &Secret) -> io::Result<()>;
    fn get(&self, key: &str) -> io::Result<Option<Secret>>;
    fn delete(&self, key: &str) -> io::Result<()>;
}

/// Keys are Minecraft profile UUIDs (32 hex chars), which also makes them
/// safe file names.
pub fn validate_key(key: &str) -> io::Result<()> {
    if key.len() == 32 && key.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid vault key",
        ))
    }
}

const MAGIC: &[u8; 4] = b"NCV1";
const ENTROPY: &[u8] = b"NightClient token vault v1";

pub struct DpapiVault {
    dir: PathBuf,
}

impl DpapiVault {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn path(&self, key: &str) -> io::Result<PathBuf> {
        validate_key(key)?;
        Ok(self.dir.join(format!("{}.ncv", key.to_ascii_lowercase())))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

impl TokenVault for DpapiVault {
    fn put(&self, key: &str, secret: &Secret) -> io::Result<()> {
        let path = self.path(key)?;
        let mut blob = MAGIC.to_vec();
        blob.extend(dpapi::protect(secret.expose().as_bytes())?);
        fsutil::atomic_write(&path, &blob)
    }

    fn get(&self, key: &str) -> io::Result<Option<Secret>> {
        let path = self.path(key)?;
        let data = match std::fs::read(&path) {
            Ok(d) => d,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let body = data
            .strip_prefix(MAGIC.as_slice())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "unknown vault format"))?;
        let plain = dpapi::unprotect(body)?;
        let s = String::from_utf8(plain)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "vault entry is not UTF-8"))?;
        Ok(Some(Secret::new(s)))
    }

    fn delete(&self, key: &str) -> io::Result<()> {
        match std::fs::remove_file(self.path(key)?) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            r => r,
        }
    }
}

/// In-memory vault for tests and for builds without OS protection.
#[derive(Default)]
pub struct MemoryVault(Mutex<HashMap<String, Secret>>);

impl TokenVault for MemoryVault {
    fn put(&self, key: &str, secret: &Secret) -> io::Result<()> {
        validate_key(key)?;
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(key.to_ascii_lowercase(), secret.clone());
        Ok(())
    }
    fn get(&self, key: &str) -> io::Result<Option<Secret>> {
        validate_key(key)?;
        Ok(self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&key.to_ascii_lowercase())
            .cloned())
    }
    fn delete(&self, key: &str) -> io::Result<()> {
        validate_key(key)?;
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&key.to_ascii_lowercase());
        Ok(())
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod dpapi {
    use std::{io, ptr};

    use super::ENTROPY;

    #[repr(C)]
    struct DataBlob {
        len: u32,
        data: *mut u8,
    }

    const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x1;

    #[link(name = "crypt32")]
    unsafe extern "system" {
        fn CryptProtectData(
            data_in: *const DataBlob,
            description: *const u16,
            entropy: *const DataBlob,
            reserved: *mut core::ffi::c_void,
            prompt: *mut core::ffi::c_void,
            flags: u32,
            data_out: *mut DataBlob,
        ) -> i32;
        fn CryptUnprotectData(
            data_in: *const DataBlob,
            description: *mut *mut u16,
            entropy: *const DataBlob,
            reserved: *mut core::ffi::c_void,
            prompt: *mut core::ffi::c_void,
            flags: u32,
            data_out: *mut DataBlob,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LocalFree(mem: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
    }

    fn blob(bytes: &[u8]) -> io::Result<DataBlob> {
        let len = u32::try_from(bytes.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "secret too large"))?;
        // DPAPI does not write through `data_in`/`entropy`; the cast to *mut
        // only satisfies the C struct layout.
        Ok(DataBlob {
            len,
            data: bytes.as_ptr().cast_mut(),
        })
    }

    /// Copy DPAPI's output into a Vec, zero it, and free it with LocalFree.
    fn take(out: DataBlob) -> Vec<u8> {
        // SAFETY: on success DPAPI returns `len` valid bytes at `data`,
        // allocated with LocalAlloc, which we own and must free exactly once.
        unsafe {
            let v = std::slice::from_raw_parts(out.data, out.len as usize).to_vec();
            ptr::write_bytes(out.data, 0, out.len as usize);
            LocalFree(out.data.cast());
            v
        }
    }

    pub fn protect(plain: &[u8]) -> io::Result<Vec<u8>> {
        let input = blob(plain)?;
        let entropy = blob(ENTROPY)?;
        let mut out = DataBlob {
            len: 0,
            data: ptr::null_mut(),
        };
        // SAFETY: all pointers reference live, correctly sized buffers for
        // the duration of the call; optional pointers are null.
        let ok = unsafe {
            CryptProtectData(
                &input,
                ptr::null(),
                &entropy,
                ptr::null_mut(),
                ptr::null_mut(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(take(out))
    }

    pub fn unprotect(cipher: &[u8]) -> io::Result<Vec<u8>> {
        let input = blob(cipher)?;
        let entropy = blob(ENTROPY)?;
        let mut out = DataBlob {
            len: 0,
            data: ptr::null_mut(),
        };
        // SAFETY: as in `protect`; the description out-pointer is null (not requested).
        let ok = unsafe {
            CryptUnprotectData(
                &input,
                ptr::null_mut(),
                &entropy,
                ptr::null_mut(),
                ptr::null_mut(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(take(out))
    }
}

#[cfg(not(windows))]
mod dpapi {
    use std::io;
    pub fn protect(_: &[u8]) -> io::Result<Vec<u8>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "DPAPI is Windows-only",
        ))
    }
    pub fn unprotect(_: &[u8]) -> io::Result<Vec<u8>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "DPAPI is Windows-only",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn dpapi_roundtrip_is_encrypted_at_rest() {
        let dir = tempfile::tempdir().unwrap();
        let v = DpapiVault::new(dir.path());
        let secret = Secret::new("M.C540_BAY.0.U.-refresh-token-value-with-unicode-ü");
        assert!(v.get(KEY).unwrap().is_none());
        v.put(KEY, &secret).unwrap();
        let raw = std::fs::read(dir.path().join(format!("{KEY}.ncv"))).unwrap();
        assert!(raw.starts_with(b"NCV1"));
        assert!(
            !raw.windows(13).any(|w| w == b"refresh-token"),
            "plaintext must not be on disk"
        );
        assert_eq!(v.get(KEY).unwrap().unwrap(), secret);
        assert_eq!(
            v.get(&KEY.to_uppercase()).unwrap().unwrap(),
            secret,
            "keys are case-insensitive"
        );
        v.delete(KEY).unwrap();
        assert!(v.get(KEY).unwrap().is_none());
        v.delete(KEY).unwrap(); // idempotent
    }

    #[test]
    fn tampered_or_foreign_data_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let v = DpapiVault::new(dir.path());
        v.put(KEY, &Secret::new("secret")).unwrap();
        let p = dir.path().join(format!("{KEY}.ncv"));
        let mut raw = std::fs::read(&p).unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 0xFF;
        std::fs::write(&p, &raw).unwrap();
        assert!(v.get(KEY).is_err(), "DPAPI integrity check fails");
        std::fs::write(&p, b"plain text").unwrap();
        assert!(v.get(KEY).is_err());
    }

    #[test]
    fn keys_are_validated() {
        let dir = tempfile::tempdir().unwrap();
        let v = DpapiVault::new(dir.path());
        for bad in ["", "../../x", "0123456789abcdef0123456789abcdeg", "a/b"] {
            assert!(v.put(bad, &Secret::new("x")).is_err(), "{bad}");
            assert!(MemoryVault::default().get(bad).is_err());
        }
    }
}
