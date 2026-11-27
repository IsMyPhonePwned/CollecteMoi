//! Data-directory size and host disk free space for the admin Settings UI.
//! Also selective wipe helpers used by Settings → Delete data.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct StorageBreakdown {
    pub name: String,
    pub path: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct StorageStats {
    /// Absolute (or configured) data directory path.
    pub data_dir: String,
    /// Total bytes under `data_dir` (sum of breakdown + residual).
    pub data_bytes: u64,
    pub breakdown: Vec<StorageBreakdown>,
    /// Host filesystem that holds `data_dir` (when available).
    pub disk_total_bytes: Option<u64>,
    pub disk_used_bytes: Option<u64>,
    pub disk_free_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct WipeRequest {
    /// Confirm string must be exactly `DELETE`.
    pub confirm: String,
    pub password: String,
    /// Uploads, blobs, jobs, analysis artefacts.
    #[serde(default)]
    pub collected: bool,
    /// Access + enrollment audit logs (and pending OTP challenges).
    #[serde(default)]
    pub logs: bool,
    /// Generated APK/IPA under data/mobile.
    #[serde(default)]
    pub mobile_builds: bool,
    /// Flutter/Gradle caches under the data volume.
    #[serde(default)]
    pub build_caches: bool,
    /// Enrolled device records (does not remove companies / lab keys / TLS / admin).
    #[serde(default)]
    pub devices: bool,
}

impl WipeRequest {
    pub fn any_selected(&self) -> bool {
        self.collected || self.logs || self.mobile_builds || self.build_caches || self.devices
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct WipeReport {
    pub ok: bool,
    pub cleared: Vec<String>,
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub details: serde_json::Value,
}

/// Remove a directory tree if it exists, then recreate an empty directory.
pub fn reset_dir(path: &Path) -> std::io::Result<()> {
    if path.exists() {
        fs::remove_dir_all(path)?;
    }
    fs::create_dir_all(path)?;
    Ok(())
}

/// Delete a single file if present (ignore missing).
pub fn remove_file_if_exists(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

pub fn collect(data_dir: &Path) -> StorageStats {
    let data_dir_disp = data_dir.display().to_string();
    let named = [
        ("Database", "collectemoi.db"),
        ("Blobs", "blobs"),
        ("Uploads", "uploads"),
        ("Mobile builds", "mobile"),
        ("Branding", "branding"),
        ("Keys", "keys"),
        ("TLS", "tls"),
        ("Logs / run", "run"),
        ("Analysis", "analysis"),
        ("Pub cache", ".pub-cache"),
        ("Gradle cache", ".gradle"),
    ];

    let mut breakdown = Vec::with_capacity(named.len() + 1);
    let mut accounted = 0u64;
    for (label, rel) in named {
        let p = data_dir.join(rel);
        if !p.exists() {
            continue;
        }
        let bytes = path_size(&p);
        if bytes == 0 && p.is_dir() {
            continue;
        }
        accounted = accounted.saturating_add(bytes);
        breakdown.push(StorageBreakdown {
            name: label.into(),
            path: rel.into(),
            bytes,
        });
    }

    let data_bytes = path_size(data_dir);
    let other = data_bytes.saturating_sub(accounted);
    if other > 0 {
        breakdown.push(StorageBreakdown {
            name: "Other".into(),
            path: ".".into(),
            bytes: other,
        });
    }
    breakdown.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(a.name.cmp(&b.name)));

    let (disk_total_bytes, disk_used_bytes, disk_free_bytes) = disk_usage(data_dir);

    StorageStats {
        data_dir: data_dir_disp,
        data_bytes,
        breakdown,
        disk_total_bytes,
        disk_used_bytes,
        disk_free_bytes,
    }
}

fn path_size(path: &Path) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return 0;
    };
    if meta.file_type().is_symlink() {
        // Do not follow symlinks (avoids counting outside the data volume).
        return 0;
    }
    if meta.is_file() {
        return meta.len();
    }
    if !meta.is_dir() {
        return 0;
    }
    let mut total = 0u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in rd.flatten() {
            let p = entry.path();
            let Ok(m) = fs::symlink_metadata(&p) else {
                continue;
            };
            if m.file_type().is_symlink() {
                continue;
            }
            if m.is_file() {
                total = total.saturating_add(m.len());
            } else if m.is_dir() {
                stack.push(p);
            }
        }
    }
    total
}

fn disk_usage(path: &Path) -> (Option<u64>, Option<u64>, Option<u64>) {
    let probe = resolve_existing(path);
    #[cfg(unix)]
    {
        return unix_disk_usage(&probe);
    }
    #[cfg(not(unix))]
    {
        let _ = probe;
        (None, None, None)
    }
}

fn resolve_existing(path: &Path) -> PathBuf {
    if path.exists() {
        return path.to_path_buf();
    }
    path.ancestors()
        .find(|p| p.exists())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("/"))
}

#[cfg(unix)]
fn unix_disk_usage(path: &Path) -> (Option<u64>, Option<u64>, Option<u64>) {
    use std::ffi::CString;
    use std::mem::MaybeUninit;
    use std::os::unix::ffi::OsStrExt;

    let c_path = match CString::new(path.as_os_str().as_bytes()) {
        Ok(c) => c,
        Err(_) => return (None, None, None),
    };
    let mut buf = MaybeUninit::<libc::statvfs>::uninit();
    let rc = unsafe { libc::statvfs(c_path.as_ptr(), buf.as_mut_ptr()) };
    if rc != 0 {
        return (None, None, None);
    }
    let st = unsafe { buf.assume_init() };
    // Field widths differ by platform (e.g. u32 on macOS, u64 on Linux).
    let frsize = st.f_frsize as u64;
    let total = (st.f_blocks as u64).saturating_mul(frsize);
    let free = (st.f_bavail as u64).saturating_mul(frsize);
    let used = total.saturating_sub((st.f_bfree as u64).saturating_mul(frsize));
    (Some(total), Some(used), Some(free))
}
