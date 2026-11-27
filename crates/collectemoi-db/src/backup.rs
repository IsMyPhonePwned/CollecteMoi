//! Export / import of the CollecteMoi catalogue + optional keys and blob bytes.
//!
//! Archive layout (zip):
//!   manifest.json
//!   collectemoi.db
//!   settings.json                 (optional runtime settings snapshot)
//!   keys/...                      (optional — case + device key files)
//!   blobs/<aa>/<bb>/<uuid>/...    (optional — blob.bin + meta.json)

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use rusqlite::Connection;
use rusqlite::backup::Backup;
use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::Db;
use crate::ids::reject_path_chars;

pub const BACKUP_FORMAT: &str = "collectemoi-backup";
pub const BACKUP_VERSION: u32 = 1;

#[derive(Debug, Clone, Default)]
pub struct ExportOptions {
    pub include_keys: bool,
    pub include_blobs: bool,
    /// Optional JSON object written as `settings.json`.
    pub settings_json: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ImportOptions {
    pub include_keys: bool,
    pub include_blobs: bool,
    /// Replace live SQLite contents (required for catalogue restore).
    pub replace_db: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupManifest {
    pub format: String,
    pub version: u32,
    pub created_at: String,
    pub include_keys: bool,
    pub include_blobs: bool,
    pub has_settings: bool,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportReport {
    pub restored_db: bool,
    pub restored_keys: usize,
    pub restored_blobs: usize,
    pub has_settings: bool,
    pub settings_json: Option<String>,
    pub manifest: BackupManifest,
}

impl Db {
    /// Checkpoint WAL and write a consistent DB copy into `dest`.
    pub fn backup_to_file(&self, dest: &Path) -> Result<()> {
        {
            let conn = self.acquire()?;
            conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        let src_path = self.path.clone();
        let src = Connection::open(&src_path)
            .with_context(|| format!("open source db {}", src_path.display()))?;
        let mut dst = Connection::open(dest)
            .with_context(|| format!("create backup db {}", dest.display()))?;
        {
            let backup = Backup::new(&src, &mut dst).context("sqlite backup init")?;
            backup
                .run_to_completion(100, Duration::from_millis(1), None)
                .context("sqlite backup copy")?;
        }
        Ok(())
    }

    /// Replace the live database contents from `src_db` using the SQLite backup API.
    pub fn restore_from_file(&self, src_db: &Path) -> Result<()> {
        let src = Connection::open(src_db)
            .with_context(|| format!("open import db {}", src_db.display()))?;
        let mut dst = self.acquire()?;
        {
            let backup = Backup::new(&src, &mut dst).context("sqlite restore init")?;
            backup
                .run_to_completion(100, Duration::from_millis(1), None)
                .context("sqlite restore copy")?;
        }
        dst.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(())
    }
}

/// Build a zip archive in memory (suitable for HTTP download).
pub fn export_zip(data_dir: &Path, db: &Db, opts: &ExportOptions) -> Result<Vec<u8>> {
    let tmp = tempfile_dir(data_dir)?;
    let db_copy = tmp.join("collectemoi.db");
    db.backup_to_file(&db_copy)?;

    let mut buf = Vec::new();
    {
        let mut zip = ZipWriter::new(std::io::Cursor::new(&mut buf));
        let opts_stored =
            SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

        let manifest = BackupManifest {
            format: BACKUP_FORMAT.into(),
            version: BACKUP_VERSION,
            created_at: now_secs_str(),
            include_keys: opts.include_keys,
            include_blobs: opts.include_blobs,
            has_settings: opts.settings_json.is_some(),
            note: "CollecteMoi lab backup — DB is authoritative; blob/key files optional.".into(),
        };
        zip.start_file("manifest.json", opts_stored)?;
        zip.write_all(serde_json::to_vec_pretty(&manifest)?.as_slice())?;

        zip.start_file("collectemoi.db", opts_stored)?;
        let mut f = File::open(&db_copy)?;
        std::io::copy(&mut f, &mut zip)?;

        if let Some(ref settings) = opts.settings_json {
            zip.start_file("settings.json", opts_stored)?;
            zip.write_all(settings.as_bytes())?;
        }

        if opts.include_keys {
            add_dir_to_zip(&mut zip, &data_dir.join("keys"), "keys", opts_stored)?;
        }
        if opts.include_blobs {
            add_dir_to_zip(&mut zip, &data_dir.join("blobs"), "blobs", opts_stored)?;
        }

        zip.finish()?;
    }

    let _ = fs::remove_dir_all(&tmp);
    Ok(buf)
}

/// Write zip to `out_path`.
pub fn export_zip_to_path(
    data_dir: &Path,
    db: &Db,
    opts: &ExportOptions,
    out_path: &Path,
) -> Result<()> {
    let bytes = export_zip(data_dir, db, opts)?;
    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(out_path, bytes)?;
    Ok(())
}

/// Import a backup zip into `data_dir` / live `db`.
pub fn import_zip(
    data_dir: &Path,
    db: &Db,
    zip_bytes: &[u8],
    opts: &ImportOptions,
) -> Result<ImportReport> {
    let staging = tempfile_dir(data_dir)?;
    extract_zip_safe(zip_bytes, &staging)?;

    let manifest_path = staging.join("manifest.json");
    if !manifest_path.is_file() {
        bail!("invalid backup: missing manifest.json");
    }
    let manifest: BackupManifest = serde_json::from_slice(&fs::read(&manifest_path)?)?;
    if manifest.format != BACKUP_FORMAT {
        bail!("invalid backup format: {}", manifest.format);
    }
    if manifest.version > BACKUP_VERSION {
        bail!(
            "backup version {} newer than supported {}",
            manifest.version,
            BACKUP_VERSION
        );
    }

    let imported_db = staging.join("collectemoi.db");
    if !imported_db.is_file() {
        bail!("invalid backup: missing collectemoi.db");
    }

    // Sanity: open and check a known table exists.
    {
        let conn = Connection::open(&imported_db)?;
        let ok: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='devices'",
            [],
            |r| r.get(0),
        )?;
        if ok == 0 {
            bail!("invalid backup database: devices table missing");
        }
    }

    let mut restored_db = false;
    if opts.replace_db {
        db.restore_from_file(&imported_db)?;
        // Also replace on-disk file for next process start.
        let live = data_dir.join("collectemoi.db");
        db.backup_to_file(&live)?;
        restored_db = true;
    }

    let mut restored_keys = 0usize;
    if opts.include_keys {
        let src = staging.join("keys");
        if src.is_dir() {
            restored_keys = copy_tree_safe(&src, &data_dir.join("keys"))?;
        }
    }

    let mut restored_blobs = 0usize;
    if opts.include_blobs {
        let src = staging.join("blobs");
        if src.is_dir() {
            restored_blobs = copy_tree_safe(&src, &data_dir.join("blobs"))?;
        }
    }

    let settings_path = staging.join("settings.json");
    let (has_settings, settings_json) = if settings_path.is_file() {
        let s = fs::read_to_string(&settings_path)?;
        (true, Some(s))
    } else {
        (false, None)
    };

    let _ = fs::remove_dir_all(&staging);

    Ok(ImportReport {
        restored_db,
        restored_keys,
        restored_blobs,
        has_settings,
        settings_json,
        manifest,
    })
}

fn tempfile_dir(data_dir: &Path) -> Result<PathBuf> {
    let base = data_dir.join("run").join("backup-tmp");
    fs::create_dir_all(&base)?;
    let name = format!("b-{}-{}", now_secs_str(), std::process::id());
    let dir = base.join(name);
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn add_dir_to_zip(
    zip: &mut ZipWriter<std::io::Cursor<&mut Vec<u8>>>,
    src: &Path,
    prefix: &str,
    options: SimpleFileOptions,
) -> Result<()> {
    if !src.is_dir() {
        return Ok(());
    }
    walk_files(src, &mut |abs, rel| {
        let name = format!("{prefix}/{}", rel.to_string_lossy().replace('\\', "/"));
        // Only allow safe relative segments.
        for comp in Path::new(&name).components() {
            match comp {
                Component::Normal(s) => {
                    let s = s.to_string_lossy();
                    if s == ".." {
                        bail!("refusing path with ..");
                    }
                }
                Component::CurDir => {}
                _ => bail!("refusing absolute or parent path in zip entry"),
            }
        }
        zip.start_file(&name, options)?;
        let mut f = File::open(abs)?;
        std::io::copy(&mut f, zip)?;
        Ok(())
    })?;
    Ok(())
}

fn extract_zip_safe(bytes: &[u8], dest: &Path) -> Result<()> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = ZipArchive::new(cursor).context("open zip")?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let name = file
            .enclosed_name()
            .ok_or_else(|| anyhow::anyhow!("zip entry has unsafe path"))?
            .to_path_buf();
        // Extra reject
        for comp in name.components() {
            if matches!(
                comp,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            ) {
                bail!("zip entry escapes archive: {}", name.display());
            }
        }
        let out = dest.join(&name);
        if file.is_dir() {
            fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut outfile = File::create(&out)?;
        std::io::copy(&mut file, &mut outfile)?;
    }
    Ok(())
}

fn copy_tree_safe(src: &Path, dest: &Path) -> Result<usize> {
    fs::create_dir_all(dest)?;
    let mut n = 0usize;
    walk_files(src, &mut |abs, rel| {
        // Validate each path segment (blob ids under blobs/ must not traverse).
        for comp in rel.components() {
            if let Component::Normal(s) = comp {
                let s = s.to_string_lossy();
                if s.contains("..") {
                    bail!("unsafe path segment");
                }
                // Allow hex shard dirs and filenames; reject separators already handled.
                let _ = reject_path_chars(&s)
                    .or_else(|_| {
                        // filenames like blob.bin / meta.json / case.pk.hex are fine
                        if s.chars()
                            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
                        {
                            Ok(())
                        } else {
                            Err(crate::ids::IdError::PathChars)
                        }
                    })
                    .map_err(|e| anyhow::anyhow!(e))?;
            } else if !matches!(comp, Component::CurDir) {
                bail!("unsafe path");
            }
        }
        let out = dest.join(rel);
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(abs, &out)?;
        n += 1;
        Ok(())
    })?;
    Ok(n)
}

fn walk_files(dir: &Path, visit: &mut dyn FnMut(&Path, &Path) -> Result<()>) -> Result<()> {
    fn walk(
        root: &Path,
        dir: &Path,
        visit: &mut dyn FnMut(&Path, &Path) -> Result<()>,
    ) -> Result<()> {
        for entry in fs::read_dir(dir)?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, visit)?;
            } else if path.is_file() {
                let rel = path.strip_prefix(root).unwrap_or(&path);
                visit(&path, rel)?;
            }
        }
        Ok(())
    }
    walk(dir, dir, visit)
}

fn now_secs_str() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_default()
}

/// Read zip bytes from a path (ctl / tests).
pub fn read_zip_file(path: &Path) -> Result<Vec<u8>> {
    let mut f = File::open(path)?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    Ok(buf)
}
