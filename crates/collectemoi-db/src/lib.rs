//! CollecteMoi SQLite catalogue.
//!
//! **Safety rules**
//! - SQL text is only built from compile-time constants / fixed allow-lists.
//! - Every user-controlled value is bound via `rusqlite::params!`.
//! - Path-like ids must pass [`ids::parse_uuid_id`] / [`ids::reject_path_chars`]
//!   before filesystem joins.
//! - Large ciphertext/plaintext files stay on disk; this DB holds metadata only.

mod backup;
mod ids;
mod migrate;
mod schema;

pub use backup::{
    BACKUP_FORMAT, BACKUP_VERSION, BackupManifest, ExportOptions, ImportOptions, ImportReport,
    export_zip, export_zip_to_path, import_zip, read_zip_file,
};
pub use ids::{IdError, like_contains, parse_uuid_id, reject_path_chars, safe_filename};
pub use migrate::ImportStats;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct Db {
    path: PathBuf,
    conn: Arc<Mutex<Connection>>,
}

impl Db {
    /// Open (or create) `<data_dir>/collectemoi.db`, migrate schema, import legacy files once.
    pub fn open(data_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        let path = data_dir.join("collectemoi.db");
        let conn =
            Connection::open(&path).with_context(|| format!("open sqlite {}", path.display()))?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA foreign_keys=ON;
             PRAGMA busy_timeout=5000;",
        )?;
        conn.execute_batch(schema::DDL)?;
        ensure_lab_keys_company_id(&conn)?;
        ensure_companies_color(&conn)?;
        {
            let applied: Option<i64> = conn
                .query_row(
                    "SELECT version FROM schema_migrations ORDER BY version DESC LIMIT 1",
                    [],
                    |r| r.get(0),
                )
                .optional()?;
            if applied != Some(schema::SCHEMA_VERSION) {
                conn.execute(
                    "INSERT OR REPLACE INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                    params![schema::SCHEMA_VERSION, now_secs_str()],
                )?;
            }
        }

        let db = Self {
            path,
            conn: Arc::new(Mutex::new(conn)),
        };
        let stats = db.import_legacy_files(data_dir)?;
        if stats.devices
            + stats.uploads
            + stats.blobs
            + stats.companies
            + stats.lab_keys
            + stats.access
            + stats.enrollment
            + stats.jobs
            > 0
        {
            tracing::info!(
                db = %db.path.display(),
                devices = stats.devices,
                uploads = stats.uploads,
                blobs = stats.blobs,
                companies = stats.companies,
                lab_keys = stats.lab_keys,
                jobs = stats.jobs,
                access = stats.access,
                enrollment = stats.enrollment,
                "imported legacy filesystem metadata into sqlite"
            );
        }
        Ok(db)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /* ── settings ─────────────────────────────────────────────── */

    pub fn setting_get(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![key],
            |r| r.get(0),
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn setting_set(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /* ── uploads ──────────────────────────────────────────────── */

    pub fn upload_upsert(&self, meta: &UploadRow) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO uploads (
                upload_id, case_id, artefact, total_size, chunk_size, chunk_count,
                ciphertext_sha256, received_json, complete, created_at, completed_at
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
             ON CONFLICT(upload_id) DO UPDATE SET
                case_id=excluded.case_id,
                artefact=excluded.artefact,
                total_size=excluded.total_size,
                chunk_size=excluded.chunk_size,
                chunk_count=excluded.chunk_count,
                ciphertext_sha256=excluded.ciphertext_sha256,
                received_json=excluded.received_json,
                complete=excluded.complete,
                created_at=excluded.created_at,
                completed_at=excluded.completed_at",
            params![
                meta.upload_id,
                meta.case_id,
                meta.artefact,
                meta.total_size as i64,
                meta.chunk_size as i64,
                meta.chunk_count as i64,
                meta.ciphertext_sha256,
                serde_json::to_string(&meta.received)?,
                if meta.complete { 1 } else { 0 },
                meta.created_at,
                meta.completed_at,
            ],
        )?;
        Ok(())
    }

    pub fn upload_get(&self, upload_id: &str) -> Result<Option<UploadRow>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT upload_id, case_id, artefact, total_size, chunk_size, chunk_count,
                    ciphertext_sha256, received_json, complete, created_at, completed_at
             FROM uploads WHERE upload_id = ?1",
            params![upload_id],
            row_to_upload,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn upload_list(&self, limit: usize, offset: usize) -> Result<(Vec<UploadRow>, usize)> {
        let limit = limit.clamp(1, 1000) as i64;
        let offset = offset as i64;
        let conn = self.conn.lock().unwrap();
        let total: usize = conn.query_row("SELECT COUNT(*) FROM uploads", [], |r| {
            r.get::<_, i64>(0).map(|n| n as usize)
        })?;
        let mut stmt = conn.prepare(
            "SELECT upload_id, case_id, artefact, total_size, chunk_size, chunk_count,
                    ciphertext_sha256, received_json, complete, created_at, completed_at
             FROM uploads
             ORDER BY created_at DESC, upload_id ASC
             LIMIT ?1 OFFSET ?2",
        )?;
        let rows = stmt
            .query_map(params![limit, offset], row_to_upload)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok((rows, total))
    }

    pub fn upload_counts(&self) -> Result<(usize, usize)> {
        let conn = self.conn.lock().unwrap();
        let total: i64 = conn.query_row("SELECT COUNT(*) FROM uploads", [], |r| r.get(0))?;
        let complete: i64 =
            conn.query_row("SELECT COUNT(*) FROM uploads WHERE complete = 1", [], |r| {
                r.get(0)
            })?;
        Ok((total as usize, complete as usize))
    }

    /* ── blobs ────────────────────────────────────────────────── */

    pub fn blob_upsert(&self, meta: &BlobRow) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO blobs (
                blob_id, upload_id, case_id, artefact, size, sha256, created_at, queued_at,
                device_public_key, filename, content_type
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
             ON CONFLICT(blob_id) DO UPDATE SET
                upload_id=excluded.upload_id,
                case_id=excluded.case_id,
                artefact=excluded.artefact,
                size=excluded.size,
                sha256=excluded.sha256,
                created_at=excluded.created_at,
                queued_at=excluded.queued_at,
                device_public_key=excluded.device_public_key,
                filename=excluded.filename,
                content_type=excluded.content_type",
            params![
                meta.blob_id,
                meta.upload_id,
                meta.case_id,
                meta.artefact,
                meta.size as i64,
                meta.sha256,
                meta.created_at,
                meta.queued_at,
                meta.device_public_key,
                meta.filename,
                meta.content_type,
            ],
        )?;
        Ok(())
    }

    pub fn blob_get(&self, blob_id: &str) -> Result<Option<BlobRow>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT blob_id, upload_id, case_id, artefact, size, sha256, created_at, queued_at,
                    device_public_key, filename, content_type
             FROM blobs WHERE blob_id = ?1",
            params![blob_id],
            row_to_blob,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn blob_delete(&self, blob_id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute("DELETE FROM blobs WHERE blob_id = ?1", params![blob_id])?;
        Ok(n > 0)
    }

    pub fn blob_list(
        &self,
        limit: usize,
        offset: usize,
        case_id: Option<&str>,
        artefact: Option<&str>,
    ) -> Result<(Vec<BlobRow>, usize)> {
        let limit = limit.clamp(1, 1000) as i64;
        let offset = offset as i64;
        let case = case_id.map(str::trim).filter(|s| !s.is_empty());
        let art = artefact.map(str::trim).filter(|s| !s.is_empty());
        let conn = self.conn.lock().unwrap();

        let (total, rows) = match (case, art) {
            (Some(c), Some(a)) => {
                let total: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM blobs WHERE case_id = ?1 AND artefact = ?2",
                    params![c, a],
                    |r| r.get(0),
                )?;
                let mut stmt = conn.prepare(
                    "SELECT blob_id, upload_id, case_id, artefact, size, sha256, created_at, queued_at,
                            device_public_key, filename, content_type
                     FROM blobs WHERE case_id = ?1 AND artefact = ?2
                     ORDER BY created_at DESC LIMIT ?3 OFFSET ?4",
                )?;
                let rows = stmt
                    .query_map(params![c, a, limit, offset], row_to_blob)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                (total, rows)
            }
            (Some(c), None) => {
                let total: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM blobs WHERE case_id = ?1",
                    params![c],
                    |r| r.get(0),
                )?;
                let mut stmt = conn.prepare(
                    "SELECT blob_id, upload_id, case_id, artefact, size, sha256, created_at, queued_at,
                            device_public_key, filename, content_type
                     FROM blobs WHERE case_id = ?1
                     ORDER BY created_at DESC LIMIT ?2 OFFSET ?3",
                )?;
                let rows = stmt
                    .query_map(params![c, limit, offset], row_to_blob)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                (total, rows)
            }
            (None, Some(a)) => {
                let total: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM blobs WHERE artefact = ?1",
                    params![a],
                    |r| r.get(0),
                )?;
                let mut stmt = conn.prepare(
                    "SELECT blob_id, upload_id, case_id, artefact, size, sha256, created_at, queued_at,
                            device_public_key, filename, content_type
                     FROM blobs WHERE artefact = ?1
                     ORDER BY created_at DESC LIMIT ?2 OFFSET ?3",
                )?;
                let rows = stmt
                    .query_map(params![a, limit, offset], row_to_blob)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                (total, rows)
            }
            (None, None) => {
                let total: i64 = conn.query_row("SELECT COUNT(*) FROM blobs", [], |r| r.get(0))?;
                let mut stmt = conn.prepare(
                    "SELECT blob_id, upload_id, case_id, artefact, size, sha256, created_at, queued_at,
                            device_public_key, filename, content_type
                     FROM blobs ORDER BY created_at DESC LIMIT ?1 OFFSET ?2",
                )?;
                let rows = stmt
                    .query_map(params![limit, offset], row_to_blob)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                (total, rows)
            }
        };
        Ok((rows, total as usize))
    }

    pub fn blob_count(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM blobs", [], |r| r.get(0))?;
        Ok(n as usize)
    }

    pub fn blobs_for_device_pubkey(&self, public_key: &str) -> Result<Vec<BlobRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT blob_id, upload_id, case_id, artefact, size, sha256, created_at, queued_at,
                    device_public_key, filename, content_type
             FROM blobs WHERE device_public_key = ?1
             ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map(params![public_key], row_to_blob)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /* ── jobs ─────────────────────────────────────────────────── */

    pub fn job_enqueue(&self, job: &JobRow) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO jobs (
                upload_id, case_id, artefact, envelope_path, device_public_key, queued_at, status
             ) VALUES (?1,?2,?3,?4,?5,?6,'pending')
             ON CONFLICT(upload_id) DO UPDATE SET
                case_id=excluded.case_id,
                artefact=excluded.artefact,
                envelope_path=excluded.envelope_path,
                device_public_key=excluded.device_public_key,
                queued_at=excluded.queued_at,
                status='pending',
                claimed_at=NULL,
                finished_at=NULL,
                error=NULL",
            params![
                job.upload_id,
                job.case_id,
                job.artefact,
                job.envelope_path,
                job.device_public_key,
                job.queued_at,
            ],
        )?;
        Ok(())
    }

    pub fn job_list_pending(&self, limit: usize) -> Result<Vec<JobRow>> {
        let limit = limit.clamp(1, 1000) as i64;
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT upload_id, case_id, artefact, envelope_path, device_public_key, queued_at, status,
                    claimed_at, finished_at, error
             FROM jobs WHERE status = 'pending'
             ORDER BY queued_at ASC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit], row_to_job)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Atomically claim a pending job. Returns the job if claimed.
    pub fn job_claim(&self, upload_id: &str) -> Result<Option<JobRow>> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute(
            "UPDATE jobs SET status = 'claimed', claimed_at = ?1
             WHERE upload_id = ?2 AND status = 'pending'",
            params![now_secs_str(), upload_id],
        )?;
        if n == 0 {
            return Ok(None);
        }
        conn.query_row(
            "SELECT upload_id, case_id, artefact, envelope_path, device_public_key, queued_at, status,
                    claimed_at, finished_at, error
             FROM jobs WHERE upload_id = ?1",
            params![upload_id],
            row_to_job,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn job_mark_done(&self, upload_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE jobs SET status = 'done', finished_at = ?1, error = NULL WHERE upload_id = ?2",
            params![now_secs_str(), upload_id],
        )?;
        Ok(())
    }

    pub fn job_mark_failed(&self, upload_id: &str, error: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE jobs SET status = 'failed', finished_at = ?1, error = ?2 WHERE upload_id = ?3",
            params![now_secs_str(), error, upload_id],
        )?;
        Ok(())
    }

    pub fn job_count_pending(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM jobs WHERE status IN ('pending','claimed')",
            [],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    pub fn job_list_all(&self, limit: usize) -> Result<Vec<JobRow>> {
        let limit = limit.clamp(1, 1000) as i64;
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT upload_id, case_id, artefact, envelope_path, device_public_key, queued_at, status,
                    claimed_at, finished_at, error
             FROM jobs ORDER BY queued_at DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit], row_to_job)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Client-visible submission history for one enrolled device.
    pub fn submissions_for_device(
        &self,
        public_key: &str,
        limit: usize,
    ) -> Result<Vec<DeviceSubmissionRow>> {
        let limit = limit.clamp(1, 500) as i64;
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT j.upload_id, j.artefact, COALESCE(u.total_size, 0), j.queued_at,
                    j.status
             FROM jobs j
             LEFT JOIN uploads u ON u.upload_id = j.upload_id
             WHERE lower(j.device_public_key) = lower(?1)
             ORDER BY CAST(j.queued_at AS INTEGER) DESC, j.upload_id ASC
             LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![public_key, limit], |r| {
                Ok(DeviceSubmissionRow {
                    upload_id: r.get(0)?,
                    artefact: r.get(1)?,
                    size_bytes: r.get::<_, i64>(2)?.max(0) as u64,
                    submitted_at: r.get(3)?,
                    status: r.get(4)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /* ── devices ──────────────────────────────────────────────── */

    pub fn device_upsert(&self, d: &DeviceRow) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO devices (
                device_id, case_id, public_key, noise_key, noise_key_fingerprint,
                platform, model, os_version, enrolled_at, banned, email, company_id
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
             ON CONFLICT(device_id) DO UPDATE SET
                case_id=excluded.case_id,
                public_key=excluded.public_key,
                noise_key=excluded.noise_key,
                noise_key_fingerprint=excluded.noise_key_fingerprint,
                platform=excluded.platform,
                model=excluded.model,
                os_version=excluded.os_version,
                enrolled_at=excluded.enrolled_at,
                banned=excluded.banned,
                email=excluded.email,
                company_id=excluded.company_id",
            params![
                d.device_id,
                d.case_id,
                d.public_key,
                d.noise_key,
                d.noise_key_fingerprint,
                d.platform,
                d.model,
                d.os_version,
                d.enrolled_at,
                if d.banned { 1 } else { 0 },
                d.email,
                d.company_id,
            ],
        )?;
        Ok(())
    }

    pub fn device_list(&self) -> Result<Vec<DeviceRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT device_id, case_id, public_key, noise_key, noise_key_fingerprint,
                    platform, model, os_version, enrolled_at, banned, email, company_id
             FROM devices ORDER BY enrolled_at ASC",
        )?;
        let rows = stmt
            .query_map([], row_to_device)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn device_get(&self, device_id: &str) -> Result<Option<DeviceRow>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT device_id, case_id, public_key, noise_key, noise_key_fingerprint,
                    platform, model, os_version, enrolled_at, banned, email, company_id
             FROM devices WHERE device_id = ?1",
            params![device_id],
            row_to_device,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn device_find_by_pubkey(&self, public_key: &str) -> Result<Option<DeviceRow>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT device_id, case_id, public_key, noise_key, noise_key_fingerprint,
                    platform, model, os_version, enrolled_at, banned, email, company_id
             FROM devices WHERE lower(public_key) = lower(?1) LIMIT 1",
            params![public_key],
            row_to_device,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn device_set_banned(&self, device_id: &str, banned: bool) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE devices SET banned = ?1 WHERE device_id = ?2",
            params![if banned { 1 } else { 0 }, device_id],
        )?;
        Ok(())
    }

    pub fn device_ban_matching_pubkey(&self, public_key: &str, banned: bool) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE devices SET banned = ?1 WHERE lower(public_key) = lower(?2)",
            params![if banned { 1 } else { 0 }, public_key],
        )?;
        Ok(())
    }

    /* ── banned keys ──────────────────────────────────────────── */

    pub fn banned_upsert(&self, public_key: &str, banned_at: &str, reason: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO banned_public_keys (public_key, banned_at, reason) VALUES (?1,?2,?3)
             ON CONFLICT(public_key) DO UPDATE SET banned_at=excluded.banned_at, reason=excluded.reason",
            params![public_key, banned_at, reason],
        )?;
        Ok(())
    }

    pub fn banned_remove(&self, public_key: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM banned_public_keys WHERE public_key = ?1",
            params![public_key],
        )?;
        Ok(())
    }

    pub fn banned_list(&self) -> Result<Vec<BannedKeyRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT public_key, banned_at, reason FROM banned_public_keys ORDER BY banned_at ASC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(BannedKeyRow {
                    public_key: r.get(0)?,
                    banned_at: r.get(1)?,
                    reason: r.get(2)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn banned_contains(&self, public_key: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM banned_public_keys WHERE lower(public_key) = lower(?1)",
            params![public_key],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    /* ── companies / lab keys ─────────────────────────────────── */

    pub fn company_upsert(&self, c: &CompanyRow) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO companies (id, name, domains_json, case_id, enabled, updated_at, color)
             VALUES (?1,?2,?3,?4,?5,?6,?7)
             ON CONFLICT(id) DO UPDATE SET
                name=excluded.name,
                domains_json=excluded.domains_json,
                case_id=excluded.case_id,
                enabled=excluded.enabled,
                updated_at=excluded.updated_at,
                color=excluded.color",
            params![
                c.id,
                c.name,
                serde_json::to_string(&c.domains)?,
                c.case_id,
                if c.enabled { 1 } else { 0 },
                c.updated_at,
                c.color,
            ],
        )?;
        Ok(())
    }

    pub fn company_list(&self) -> Result<Vec<CompanyRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, domains_json, case_id, enabled, updated_at, color FROM companies ORDER BY id",
        )?;
        let rows = stmt
            .query_map([], row_to_company)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn company_get(&self, id: &str) -> Result<Option<CompanyRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, domains_json, case_id, enabled, updated_at, color FROM companies WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id], row_to_company)?;
        Ok(rows.next().transpose()?)
    }

    pub fn company_delete(&self, id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute("DELETE FROM companies WHERE id = ?1", params![id])?;
        Ok(n > 0)
    }

    pub fn lab_key_upsert(&self, k: &LabKeyRow) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO lab_keys (key, label, email, case_id, reusable, enabled, updated_at, company_id)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
             ON CONFLICT(key) DO UPDATE SET
                label=excluded.label,
                email=excluded.email,
                case_id=excluded.case_id,
                reusable=excluded.reusable,
                enabled=excluded.enabled,
                updated_at=excluded.updated_at,
                company_id=excluded.company_id",
            params![
                k.key,
                k.label,
                k.email,
                k.case_id,
                if k.reusable { 1 } else { 0 },
                if k.enabled { 1 } else { 0 },
                k.updated_at,
                k.company_id,
            ],
        )?;
        Ok(())
    }

    pub fn lab_key_list(&self) -> Result<Vec<LabKeyRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT key, label, email, case_id, reusable, enabled, updated_at, company_id FROM lab_keys ORDER BY key",
        )?;
        let rows = stmt
            .query_map([], row_to_lab_key)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn lab_key_get(&self, key: &str) -> Result<Option<LabKeyRow>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT key, label, email, case_id, reusable, enabled, updated_at, company_id
             FROM lab_keys WHERE key = ?1",
            params![key],
            row_to_lab_key,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn lab_key_delete(&self, key: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute("DELETE FROM lab_keys WHERE key = ?1", params![key])?;
        Ok(n > 0)
    }

    /* ── access / enrollment logs ─────────────────────────────── */

    pub fn access_record(&self, e: &AccessEventRow) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO access_events (
                ts, method, path, status, duration_ms, remote, user_agent, device_id, kind, resource_id
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![
                e.ts,
                e.method,
                e.path,
                e.status,
                e.duration_ms,
                e.remote,
                e.user_agent,
                e.device_id,
                e.kind,
                e.resource_id,
            ],
        )?;
        Ok(())
    }

    pub fn access_list(
        &self,
        limit: usize,
        kind: Option<&str>,
        remote_substr: Option<&str>,
    ) -> Result<Vec<AccessEventRow>> {
        let limit = limit.clamp(1, 1000) as i64;
        let kind = kind.map(str::trim).filter(|s| !s.is_empty());
        let remote = remote_substr.map(str::trim).filter(|s| !s.is_empty());
        let conn = self.conn.lock().unwrap();

        match (kind, remote) {
            (Some(k), Some(r)) => {
                let pattern = like_contains(r);
                let mut stmt = conn.prepare(
                    "SELECT ts, method, path, status, duration_ms, remote, user_agent, device_id, kind, resource_id
                     FROM access_events
                     WHERE kind = ?1 AND remote LIKE ?2 ESCAPE '\\'
                     ORDER BY id DESC LIMIT ?3",
                )?;
                let rows = stmt
                    .query_map(params![k, pattern, limit], row_to_access)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                Ok(rows)
            }
            (Some(k), None) => {
                let mut stmt = conn.prepare(
                    "SELECT ts, method, path, status, duration_ms, remote, user_agent, device_id, kind, resource_id
                     FROM access_events WHERE kind = ?1
                     ORDER BY id DESC LIMIT ?2",
                )?;
                let rows = stmt
                    .query_map(params![k, limit], row_to_access)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                Ok(rows)
            }
            (None, Some(r)) => {
                let pattern = like_contains(r);
                let mut stmt = conn.prepare(
                    "SELECT ts, method, path, status, duration_ms, remote, user_agent, device_id, kind, resource_id
                     FROM access_events
                     WHERE remote LIKE ?1 ESCAPE '\\'
                     ORDER BY id DESC LIMIT ?2",
                )?;
                let rows = stmt
                    .query_map(params![pattern, limit], row_to_access)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                Ok(rows)
            }
            (None, None) => {
                let mut stmt = conn.prepare(
                    "SELECT ts, method, path, status, duration_ms, remote, user_agent, device_id, kind, resource_id
                     FROM access_events ORDER BY id DESC LIMIT ?1",
                )?;
                let rows = stmt
                    .query_map(params![limit], row_to_access)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                Ok(rows)
            }
        }
    }

    pub fn access_for_resource(
        &self,
        resource_id: &str,
        limit: usize,
    ) -> Result<Vec<AccessEventRow>> {
        let limit = limit.clamp(1, 500) as i64;
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT ts, method, path, status, duration_ms, remote, user_agent, device_id, kind, resource_id
             FROM access_events WHERE resource_id = ?1
             ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![resource_id, limit], row_to_access)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn enrollment_log(&self, e: &EnrollmentEventRow) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO enrollment_events (
                ts, event, email, company_id, challenge_id, device_id, lab_key, detail
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                e.ts as i64,
                e.event,
                e.email,
                e.company_id,
                e.challenge_id,
                e.device_id,
                e.lab_key,
                e.detail,
            ],
        )?;
        Ok(())
    }

    pub fn enrollment_list(&self, limit: usize) -> Result<Vec<EnrollmentEventRow>> {
        let limit = limit.clamp(1, 1000) as i64;
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT ts, event, email, company_id, challenge_id, device_id, lab_key, detail
             FROM enrollment_events ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit], |r| {
                Ok(EnrollmentEventRow {
                    ts: r.get::<_, i64>(0)? as u64,
                    event: r.get(1)?,
                    email: r.get(2)?,
                    company_id: r.get(3)?,
                    challenge_id: r.get(4)?,
                    device_id: r.get(5)?,
                    lab_key: r.get(6)?,
                    detail: r.get(7)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /* ── OTP challenges (durable) ─────────────────────────────── */

    pub fn challenge_upsert(&self, c: &ChallengeRow) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO otp_challenges (
                id, email, company_id, case_id, code_hash, code_plain, expires_at, used, via_lab_key
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
             ON CONFLICT(id) DO UPDATE SET
                email=excluded.email,
                company_id=excluded.company_id,
                case_id=excluded.case_id,
                code_hash=excluded.code_hash,
                code_plain=excluded.code_plain,
                expires_at=excluded.expires_at,
                used=excluded.used,
                via_lab_key=excluded.via_lab_key",
            params![
                c.id,
                c.email,
                c.company_id,
                c.case_id,
                c.code_hash,
                c.code_plain,
                c.expires_at as i64,
                if c.used { 1 } else { 0 },
                c.via_lab_key,
            ],
        )?;
        Ok(())
    }

    pub fn challenge_get(&self, id: &str) -> Result<Option<ChallengeRow>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, email, company_id, case_id, code_hash, code_plain, expires_at, used, via_lab_key
             FROM otp_challenges WHERE id = ?1",
            params![id],
            row_to_challenge,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn challenge_mark_used(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE otp_challenges SET used = 1 WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    /* ── admin auth ───────────────────────────────────────────── */

    pub fn admin_account_get(&self) -> Result<Option<AdminAccountRow>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT username, password_hash, totp_secret, totp_enabled, created_at, updated_at
             FROM admin_account WHERE id = 1",
            [],
            |r| {
                Ok(AdminAccountRow {
                    username: r.get(0)?,
                    password_hash: r.get(1)?,
                    totp_secret: r.get(2)?,
                    totp_enabled: r.get::<_, i64>(3)? != 0,
                    created_at: r.get(4)?,
                    updated_at: r.get(5)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn admin_account_upsert(&self, row: &AdminAccountRow) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO admin_account (id, username, password_hash, totp_secret, totp_enabled, created_at, updated_at)
             VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
                username=excluded.username,
                password_hash=excluded.password_hash,
                totp_secret=excluded.totp_secret,
                totp_enabled=excluded.totp_enabled,
                updated_at=excluded.updated_at",
            params![
                row.username,
                row.password_hash,
                row.totp_secret,
                if row.totp_enabled { 1 } else { 0 },
                row.created_at,
                row.updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn admin_session_insert(&self, s: &AdminSessionRow) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO admin_sessions (
                token_hash, stage, created_at, expires_at, last_seen_at, remote, user_agent
             ) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                s.token_hash,
                s.stage,
                s.created_at,
                s.expires_at,
                s.last_seen_at,
                s.remote,
                s.user_agent,
            ],
        )?;
        Ok(())
    }

    pub fn admin_session_get(&self, token_hash: &str) -> Result<Option<AdminSessionRow>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT token_hash, stage, created_at, expires_at, last_seen_at, remote, user_agent
             FROM admin_sessions WHERE token_hash = ?1",
            params![token_hash],
            |r| {
                Ok(AdminSessionRow {
                    token_hash: r.get(0)?,
                    stage: r.get(1)?,
                    created_at: r.get(2)?,
                    expires_at: r.get(3)?,
                    last_seen_at: r.get(4)?,
                    remote: r.get(5)?,
                    user_agent: r.get(6)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn admin_session_touch(&self, token_hash: &str, last_seen_at: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE admin_sessions SET last_seen_at = ?2 WHERE token_hash = ?1",
            params![token_hash, last_seen_at],
        )?;
        Ok(())
    }

    pub fn admin_session_set_stage(
        &self,
        token_hash: &str,
        stage: &str,
        expires_at: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE admin_sessions SET stage = ?2, expires_at = ?3, last_seen_at = ?3 WHERE token_hash = ?1",
            params![token_hash, stage, expires_at],
        )?;
        Ok(())
    }

    pub fn admin_session_delete(&self, token_hash: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM admin_sessions WHERE token_hash = ?1",
            params![token_hash],
        )?;
        Ok(())
    }

    pub fn admin_sessions_purge_expired(&self, now: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM admin_sessions WHERE expires_at < ?1",
            params![now],
        )?;
        conn.execute(
            "DELETE FROM admin_webauthn_state WHERE expires_at < ?1",
            params![now],
        )?;
        Ok(())
    }

    /// Drop every admin session (password reset / lockout recovery).
    pub fn admin_sessions_clear_all(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute("DELETE FROM admin_sessions", [])?;
        Ok(n)
    }

    /// Remove all registered security keys.
    pub fn admin_webauthn_clear_all(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute("DELETE FROM admin_webauthn", [])?;
        let _ = conn.execute("DELETE FROM admin_webauthn_state", [])?;
        Ok(n)
    }

    /// Delete the singleton admin account (forces first-run setup).
    pub fn admin_account_delete(&self) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute("DELETE FROM admin_account WHERE id = 1", [])?;
        Ok(n > 0)
    }

    pub fn admin_webauthn_list(&self) -> Result<Vec<AdminWebauthnRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT cred_id, passkey_json, label, created_at, last_used_at
             FROM admin_webauthn ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(AdminWebauthnRow {
                    cred_id: r.get(0)?,
                    passkey_json: r.get(1)?,
                    label: r.get(2)?,
                    created_at: r.get(3)?,
                    last_used_at: r.get(4)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn admin_webauthn_upsert(&self, row: &AdminWebauthnRow) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO admin_webauthn (cred_id, passkey_json, label, created_at, last_used_at)
             VALUES (?1,?2,?3,?4,?5)
             ON CONFLICT(cred_id) DO UPDATE SET
                passkey_json=excluded.passkey_json,
                label=excluded.label,
                last_used_at=excluded.last_used_at",
            params![
                row.cred_id,
                row.passkey_json,
                row.label,
                row.created_at,
                row.last_used_at,
            ],
        )?;
        Ok(())
    }

    pub fn admin_webauthn_delete(&self, cred_id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute(
            "DELETE FROM admin_webauthn WHERE cred_id = ?1",
            params![cred_id],
        )?;
        Ok(n > 0)
    }

    pub fn admin_webauthn_touch(&self, cred_id: &str, last_used_at: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE admin_webauthn SET last_used_at = ?2 WHERE cred_id = ?1",
            params![cred_id, last_used_at],
        )?;
        Ok(())
    }

    pub fn admin_state_put(
        &self,
        id: &str,
        kind: &str,
        state_json: &str,
        expires_at: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO admin_webauthn_state (id, kind, state_json, expires_at)
             VALUES (?1,?2,?3,?4)
             ON CONFLICT(id) DO UPDATE SET
                kind=excluded.kind,
                state_json=excluded.state_json,
                expires_at=excluded.expires_at",
            params![id, kind, state_json, expires_at],
        )?;
        Ok(())
    }

    pub fn admin_state_take(&self, id: &str) -> Result<Option<(String, String)>> {
        let conn = self.conn.lock().unwrap();
        let row: Option<(String, String, String)> = conn
            .query_row(
                "SELECT kind, state_json, expires_at FROM admin_webauthn_state WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((kind, state_json, expires_at)) = row else {
            return Ok(None);
        };
        conn.execute(
            "DELETE FROM admin_webauthn_state WHERE id = ?1",
            params![id],
        )?;
        let now = now_secs_str();
        if expires_at < now {
            return Ok(None);
        }
        Ok(Some((kind, state_json)))
    }

    pub fn admin_state_get(&self, id: &str) -> Result<Option<(String, String)>> {
        let conn = self.conn.lock().unwrap();
        let row: Option<(String, String, String)> = conn
            .query_row(
                "SELECT kind, state_json, expires_at FROM admin_webauthn_state WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((kind, state_json, expires_at)) = row else {
            return Ok(None);
        };
        let now = now_secs_str();
        if expires_at < now {
            let _ = conn.execute(
                "DELETE FROM admin_webauthn_state WHERE id = ?1",
                params![id],
            );
            return Ok(None);
        }
        Ok(Some((kind, state_json)))
    }

    pub fn admin_state_delete(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM admin_webauthn_state WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }
}

/* ── row types ────────────────────────────────────────────────── */

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadRow {
    pub upload_id: String,
    pub case_id: String,
    pub artefact: String,
    pub total_size: u64,
    pub chunk_size: u64,
    pub chunk_count: u32,
    pub ciphertext_sha256: String,
    pub received: Vec<bool>,
    pub complete: bool,
    pub created_at: String,
    pub completed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobRow {
    pub blob_id: String,
    pub upload_id: String,
    pub case_id: String,
    pub artefact: String,
    pub size: u64,
    pub sha256: String,
    pub created_at: String,
    pub queued_at: String,
    pub device_public_key: String,
    pub filename: String,
    pub content_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRow {
    pub upload_id: String,
    pub case_id: String,
    pub artefact: String,
    pub envelope_path: String,
    pub device_public_key: String,
    pub queued_at: String,
    pub status: String,
    pub claimed_at: Option<String>,
    pub finished_at: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceSubmissionRow {
    pub upload_id: String,
    pub artefact: String,
    pub size_bytes: u64,
    pub submitted_at: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceRow {
    pub device_id: String,
    pub case_id: String,
    pub public_key: String,
    pub noise_key: String,
    pub noise_key_fingerprint: String,
    pub platform: String,
    pub model: String,
    pub os_version: String,
    pub enrolled_at: String,
    pub banned: bool,
    pub email: String,
    pub company_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BannedKeyRow {
    pub public_key: String,
    pub banned_at: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanyRow {
    pub id: String,
    pub name: String,
    pub domains: Vec<String>,
    pub case_id: String,
    pub enabled: bool,
    pub updated_at: String,
    /// Label color as `#RRGGBB` (empty = UI/default palette).
    #[serde(default)]
    pub color: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabKeyRow {
    pub key: String,
    pub label: String,
    pub email: String,
    pub case_id: String,
    pub reusable: bool,
    pub enabled: bool,
    pub updated_at: String,
    #[serde(default)]
    pub company_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessEventRow {
    pub ts: String,
    pub method: String,
    pub path: String,
    pub status: i32,
    pub duration_ms: i64,
    pub remote: String,
    pub user_agent: String,
    pub device_id: String,
    pub kind: String,
    pub resource_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrollmentEventRow {
    pub ts: u64,
    pub event: String,
    pub email: Option<String>,
    pub company_id: Option<String>,
    pub challenge_id: Option<String>,
    pub device_id: Option<String>,
    pub lab_key: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChallengeRow {
    pub id: String,
    pub email: String,
    pub company_id: String,
    pub case_id: String,
    pub code_hash: String,
    pub code_plain: Option<String>,
    pub expires_at: u64,
    pub used: bool,
    pub via_lab_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminAccountRow {
    pub username: String,
    pub password_hash: String,
    pub totp_secret: String,
    pub totp_enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminSessionRow {
    pub token_hash: String,
    pub stage: String,
    pub created_at: String,
    pub expires_at: String,
    pub last_seen_at: String,
    pub remote: String,
    pub user_agent: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminWebauthnRow {
    pub cred_id: String,
    pub passkey_json: String,
    pub label: String,
    pub created_at: String,
    pub last_used_at: Option<String>,
}

/* ── row mappers ──────────────────────────────────────────────── */

fn row_to_upload(r: &rusqlite::Row<'_>) -> rusqlite::Result<UploadRow> {
    let received_json: String = r.get(7)?;
    let received: Vec<bool> = serde_json::from_str(&received_json).unwrap_or_default();
    Ok(UploadRow {
        upload_id: r.get(0)?,
        case_id: r.get(1)?,
        artefact: r.get(2)?,
        total_size: r.get::<_, i64>(3)? as u64,
        chunk_size: r.get::<_, i64>(4)? as u64,
        chunk_count: r.get::<_, i64>(5)? as u32,
        ciphertext_sha256: r.get(6)?,
        received,
        complete: r.get::<_, i64>(8)? != 0,
        created_at: r.get(9)?,
        completed_at: r.get(10)?,
    })
}

fn row_to_blob(r: &rusqlite::Row<'_>) -> rusqlite::Result<BlobRow> {
    Ok(BlobRow {
        blob_id: r.get(0)?,
        upload_id: r.get(1)?,
        case_id: r.get(2)?,
        artefact: r.get(3)?,
        size: r.get::<_, i64>(4)? as u64,
        sha256: r.get(5)?,
        created_at: r.get(6)?,
        queued_at: r.get(7)?,
        device_public_key: r.get(8)?,
        filename: r.get(9)?,
        content_type: r.get(10)?,
    })
}

fn row_to_job(r: &rusqlite::Row<'_>) -> rusqlite::Result<JobRow> {
    Ok(JobRow {
        upload_id: r.get(0)?,
        case_id: r.get(1)?,
        artefact: r.get(2)?,
        envelope_path: r.get(3)?,
        device_public_key: r.get(4)?,
        queued_at: r.get(5)?,
        status: r.get(6)?,
        claimed_at: r.get(7)?,
        finished_at: r.get(8)?,
        error: r.get(9)?,
    })
}

fn row_to_device(r: &rusqlite::Row<'_>) -> rusqlite::Result<DeviceRow> {
    Ok(DeviceRow {
        device_id: r.get(0)?,
        case_id: r.get(1)?,
        public_key: r.get(2)?,
        noise_key: r.get(3)?,
        noise_key_fingerprint: r.get(4)?,
        platform: r.get(5)?,
        model: r.get(6)?,
        os_version: r.get(7)?,
        enrolled_at: r.get(8)?,
        banned: r.get::<_, i64>(9)? != 0,
        email: r.get(10)?,
        company_id: r.get(11)?,
    })
}

fn row_to_company(r: &rusqlite::Row<'_>) -> rusqlite::Result<CompanyRow> {
    let domains_json: String = r.get(2)?;
    let domains: Vec<String> = serde_json::from_str(&domains_json).unwrap_or_default();
    Ok(CompanyRow {
        id: r.get(0)?,
        name: r.get(1)?,
        domains,
        case_id: r.get(3)?,
        enabled: r.get::<_, i64>(4)? != 0,
        updated_at: r.get(5)?,
        color: r.get(6)?,
    })
}

/// Soft-migrate existing DBs created before schema v4 (companies.color).
fn ensure_companies_color(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(companies)")?;
    let cols: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if !cols.iter().any(|c| c == "color") {
        conn.execute(
            "ALTER TABLE companies ADD COLUMN color TEXT NOT NULL DEFAULT ''",
            [],
        )?;
    }
    Ok(())
}

fn row_to_lab_key(r: &rusqlite::Row<'_>) -> rusqlite::Result<LabKeyRow> {
    Ok(LabKeyRow {
        key: r.get(0)?,
        label: r.get(1)?,
        email: r.get(2)?,
        case_id: r.get(3)?,
        reusable: r.get::<_, i64>(4)? != 0,
        enabled: r.get::<_, i64>(5)? != 0,
        updated_at: r.get(6)?,
        company_id: r.get(7)?,
    })
}

/// Soft-migrate existing DBs created before schema v3 (lab_keys.company_id).
fn ensure_lab_keys_company_id(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(lab_keys)")?;
    let cols: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if !cols.iter().any(|c| c == "company_id") {
        conn.execute(
            "ALTER TABLE lab_keys ADD COLUMN company_id TEXT NOT NULL DEFAULT ''",
            [],
        )?;
    }
    conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_lab_keys_company ON lab_keys(company_id);")?;
    Ok(())
}

fn row_to_access(r: &rusqlite::Row<'_>) -> rusqlite::Result<AccessEventRow> {
    Ok(AccessEventRow {
        ts: r.get(0)?,
        method: r.get(1)?,
        path: r.get(2)?,
        status: r.get(3)?,
        duration_ms: r.get(4)?,
        remote: r.get(5)?,
        user_agent: r.get(6)?,
        device_id: r.get(7)?,
        kind: r.get(8)?,
        resource_id: r.get(9)?,
    })
}

fn row_to_challenge(r: &rusqlite::Row<'_>) -> rusqlite::Result<ChallengeRow> {
    Ok(ChallengeRow {
        id: r.get(0)?,
        email: r.get(1)?,
        company_id: r.get(2)?,
        case_id: r.get(3)?,
        code_hash: r.get(4)?,
        code_plain: r.get(5)?,
        expires_at: r.get::<_, i64>(6)? as u64,
        used: r.get::<_, i64>(7)? != 0,
        via_lab_key: r.get(8)?,
    })
}

fn now_secs_str() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_default()
}
