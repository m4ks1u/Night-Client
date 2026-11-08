//! Host information for memory recommendations and diagnostics.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemInfo {
    pub total_memory_mb: u64,
    pub available_memory_mb: u64,
    pub os_version: String,
    pub cpu_threads: usize,
}

pub fn system_info() -> SystemInfo {
    let (total, avail) = memory_mb().unwrap_or((0, 0));
    SystemInfo {
        total_memory_mb: total,
        available_memory_mb: avail,
        os_version: os_version(),
        cpu_threads: std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1),
    }
}

/// Memory to suggest for a new instance: a quarter of physical RAM, clamped
/// to 2–8 GiB and rounded to 512 MiB. Never "all available RAM".
pub fn recommended_memory_mb(total_mb: u64) -> u32 {
    if total_mb == 0 {
        return 4096;
    }
    let quarter = (total_mb / 4).clamp(2048, 8192);
    ((quarter / 512) * 512) as u32
}

/// Upper bound offered in the UI: physical RAM minus 2 GiB for Windows and
/// the launcher, capped at 32 GiB.
pub fn max_memory_mb(total_mb: u64) -> u32 {
    if total_mb == 0 {
        return 16384;
    }
    (total_mb.saturating_sub(2048).clamp(1024, 32768) / 512 * 512) as u32
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn memory_mb() -> Option<(u64, u64)> {
    #[repr(C)]
    struct MemoryStatusEx {
        length: u32,
        memory_load: u32,
        total_phys: u64,
        avail_phys: u64,
        total_page_file: u64,
        avail_page_file: u64,
        total_virtual: u64,
        avail_virtual: u64,
        avail_extended_virtual: u64,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalMemoryStatusEx(buf: *mut MemoryStatusEx) -> i32;
    }
    let mut s = MemoryStatusEx {
        length: std::mem::size_of::<MemoryStatusEx>() as u32,
        memory_load: 0,
        total_phys: 0,
        avail_phys: 0,
        total_page_file: 0,
        avail_page_file: 0,
        total_virtual: 0,
        avail_virtual: 0,
        avail_extended_virtual: 0,
    };
    // SAFETY: `s` is a correctly sized, initialised MEMORYSTATUSEX with
    // `dwLength` set, as the API requires.
    let ok = unsafe { GlobalMemoryStatusEx(&mut s) };
    (ok != 0).then_some((s.total_phys / 1_048_576, s.avail_phys / 1_048_576))
}

#[cfg(not(windows))]
fn memory_mb() -> Option<(u64, u64)> {
    None
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn os_version() -> String {
    #[repr(C)]
    struct OsVersionInfoW {
        size: u32,
        major: u32,
        minor: u32,
        build: u32,
        platform: u32,
        csd: [u16; 128],
    }
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn RtlGetVersion(info: *mut OsVersionInfoW) -> i32;
    }
    let mut v = OsVersionInfoW {
        size: std::mem::size_of::<OsVersionInfoW>() as u32,
        major: 0,
        minor: 0,
        build: 0,
        platform: 0,
        csd: [0; 128],
    };
    // SAFETY: `v` is a correctly sized RTL_OSVERSIONINFOW with the size field
    // set; RtlGetVersion only writes within it.
    let status = unsafe { RtlGetVersion(&mut v) };
    if status != 0 {
        return "Windows (unknown version)".into();
    }
    let name = if v.major == 10 && v.build >= 22000 {
        "Windows 11"
    } else if v.major == 10 {
        "Windows 10"
    } else {
        "Windows"
    };
    format!("{name} ({}.{}.{})", v.major, v.minor, v.build)
}

#[cfg(not(windows))]
fn os_version() -> String {
    std::env::consts::OS.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_recommendations_are_conservative() {
        assert_eq!(recommended_memory_mb(16 * 1024), 4096);
        assert_eq!(recommended_memory_mb(4 * 1024), 2048);
        assert_eq!(recommended_memory_mb(128 * 1024), 8192);
        assert_eq!(recommended_memory_mb(0), 4096);
        assert_eq!(max_memory_mb(16 * 1024), 14336);
        assert_eq!(max_memory_mb(2 * 1024), 1024);
        assert_eq!(max_memory_mb(256 * 1024), 32768);
    }

    #[test]
    fn reports_real_host_values() {
        let s = system_info();
        assert!(s.total_memory_mb > 512);
        assert!(s.os_version.starts_with("Windows"));
    }
}
