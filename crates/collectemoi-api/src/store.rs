use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use collectemoi_db::{Db, UploadRow};
use collectemoi_proto::ArtefactKind;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadMeta {
    pub upload_id: Uuid,
    pub case_id: Uuid,
    pub artefact: ArtefactKind,
    pub total_size: u64,
    pub chunk_size: u64,
    pub chunk_count: u32,
    pub ciphertext_sha256: String,
    pub received: Vec<bool>,
    pub complete: bool,
    /// Unix seconds as decimal string (set on create).
    #[serde(default)]
    pub created_at: String,
    /// Unix seconds as decimal string (set when assemble completes).
    #[serde(default)]
    pub completed_at: String,
}

/// Concurrent-safe upload store.
///
/// Chunks / envelope.bin stay on disk under `uploads/<aa>/<bb>/<uuid>/`.
/// Upload metadata is authoritative in SQLite (`collectemoi.db`).
pub struct UploadStore {
    root: PathBuf,
    db: Arc<Db>,
    locks: Mutex<HashMap<Uuid, Arc<Mutex<()>>>>,
}

impl UploadStore {
    pub fn open(root: &Path, db: Arc<Db>) -> Result<Self> {
        fs::create_dir_all(root)?;
        fs::create_dir_all(root.join("uploads"))?;
        Ok(Self {
            root: root.to_path_buf(),
            db,
            locks: Mutex::new(HashMap::new()),
        })
    }

    fn lock_for(&self, id: Uuid) -> Arc<Mutex<()>> {
        let mut map = self.locks.lock().unwrap();
        map.entry(id)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    fn shard_dir(&self, id: Uuid) -> PathBuf {
        let s = id.to_string();
        self.root
            .join("uploads")
            .join(&s[0..2])
            .join(&s[2..4])
            .join(&s)
    }

    /// Prefer sharded layout; fall back to legacy flat `uploads/<uuid>/`.
    fn upload_dir(&self, id: Uuid) -> PathBuf {
        let sharded = self.shard_dir(id);
        if sharded.exists() {
            return sharded;
        }
        let flat = self.root.join("uploads").join(id.to_string());
        if flat.exists() {
            return flat;
        }
        sharded
    }

    fn meta_path(&self, id: Uuid) -> PathBuf {
        self.upload_dir(id).join("meta.json")
    }

    fn chunk_path(&self, id: Uuid, n: u32) -> PathBuf {
        self.upload_dir(id).join(format!("chunk-{n:06}"))
    }

    fn envelope_path(&self, id: Uuid) -> PathBuf {
        self.upload_dir(id).join("envelope.bin")
    }

    pub fn create(&self, meta: UploadMeta) -> Result<()> {
        let lock = self.lock_for(meta.upload_id);
        let _g = lock.lock().unwrap();
        if self.db.upload_get(&meta.upload_id.to_string())?.is_some() {
            bail!("upload already exists");
        }
        let dir = self.shard_dir(meta.upload_id);
        if dir.exists()
            || self
                .root
                .join("uploads")
                .join(meta.upload_id.to_string())
                .exists()
        {
            bail!("upload already exists");
        }
        fs::create_dir_all(&dir)?;
        self.persist_meta(&meta)?;
        Ok(())
    }

    pub fn get(&self, id: Uuid) -> Result<UploadMeta> {
        if let Some(row) = self.db.upload_get(&id.to_string())? {
            return Ok(row_to_meta(row)?);
        }
        // Legacy fallback while migrating.
        let path = self.meta_path(id);
        let bytes = fs::read(&path).with_context(|| format!("upload {id} not found"))?;
        let meta: UploadMeta = serde_json::from_slice(&bytes)?;
        let _ = self.db.upload_upsert(&meta_to_row(&meta));
        Ok(meta)
    }

    fn persist_meta(&self, meta: &UploadMeta) -> Result<()> {
        self.db.upload_upsert(&meta_to_row(meta))?;
        // Keep a mirror meta.json for operators inspecting the tree.
        let dir = self.upload_dir(meta.upload_id);
        fs::create_dir_all(&dir)?;
        let path = dir.join("meta.json");
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(meta)?)?;
        fs::rename(tmp, path)?;
        Ok(())
    }

    fn write_meta(&self, meta: &UploadMeta) -> Result<()> {
        self.persist_meta(meta)
    }

    /// Idempotent chunk put. Verifies Digest SHA-256 when provided.
    pub fn put_chunk(
        &self,
        id: Uuid,
        n: u32,
        body: &[u8],
        digest_hex: Option<&str>,
    ) -> Result<UploadMeta> {
        let lock = self.lock_for(id);
        let _g = lock.lock().unwrap();
        let mut meta = self.get(id)?;
        if meta.complete {
            bail!("upload already complete");
        }
        if n >= meta.chunk_count {
            bail!("chunk index out of range");
        }

        let expected_len = if n + 1 == meta.chunk_count {
            let prior = meta.chunk_size * (meta.chunk_count as u64 - 1);
            meta.total_size.saturating_sub(prior)
        } else {
            meta.chunk_size
        };
        if body.len() as u64 != expected_len {
            bail!(
                "chunk length mismatch: got {}, expected {expected_len}",
                body.len()
            );
        }

        let mut hasher = Sha256::new();
        hasher.update(body);
        let actual = hex::encode(hasher.finalize());
        if let Some(expected) = digest_hex {
            let expected = expected
                .strip_prefix("sha-256=")
                .or_else(|| expected.strip_prefix("SHA-256="))
                .unwrap_or(expected);
            if !expected.eq_ignore_ascii_case(&actual) {
                bail!("chunk digest mismatch");
            }
        }

        let path = self.chunk_path(id, n);
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, body)?;
        fs::rename(tmp, path)?;

        meta.received[n as usize] = true;
        self.write_meta(&meta)?;
        Ok(meta)
    }

    /// Assemble chunks, verify whole-file hash, write envelope.bin.
    pub fn assemble(&self, id: Uuid) -> Result<(UploadMeta, PathBuf)> {
        let lock = self.lock_for(id);
        let _g = lock.lock().unwrap();
        let mut meta = self.get(id)?;
        if meta.received.iter().any(|r| !*r) {
            let missing: Vec<_> = meta
                .received
                .iter()
                .enumerate()
                .filter_map(|(i, r)| (!r).then_some(i))
                .collect();
            bail!("missing chunks: {missing:?}");
        }

        let out = self.envelope_path(id);
        let tmp = out.with_extension("bin.tmp");
        {
            let mut file = fs::File::create(&tmp)?;
            let mut hasher = Sha256::new();
            for n in 0..meta.chunk_count {
                let bytes = fs::read(self.chunk_path(id, n))?;
                hasher.update(&bytes);
                file.write_all(&bytes)?;
            }
            let actual = hex::encode(hasher.finalize());
            if !actual.eq_ignore_ascii_case(&meta.ciphertext_sha256) {
                let _ = fs::remove_file(&tmp);
                bail!("envelope SHA-256 mismatch");
            }
            file.sync_all()?;
        }
        fs::rename(tmp, &out)?;
        meta.complete = true;
        if meta.completed_at.is_empty() {
            meta.completed_at = unix_secs_str();
        }
        self.write_meta(&meta)?;
        Ok((meta, out))
    }

    pub fn read_envelope(&self, id: Uuid) -> Result<Vec<u8>> {
        Ok(fs::read(self.envelope_path(id))?)
    }

    /// Paginated list (newest-first).
    pub fn list_page(&self, limit: usize, offset: usize) -> Result<(Vec<UploadMeta>, usize)> {
        let (rows, total) = self.db.upload_list(limit, offset)?;
        let mut metas = Vec::with_capacity(rows.len());
        for row in rows {
            metas.push(row_to_meta(row)?);
        }
        if total == 0 {
            // Empty DB: fall back to filesystem once, then populate SQL.
            let mut out = Vec::new();
            walk_upload_metas(&self.root.join("uploads"), &mut out)?;
            for m in &out {
                let _ = self.db.upload_upsert(&meta_to_row(m));
            }
            out.sort_by(|a, b| {
                b.created_at
                    .cmp(&a.created_at)
                    .then_with(|| a.upload_id.cmp(&b.upload_id))
            });
            let total = out.len();
            let page = out
                .into_iter()
                .skip(offset)
                .take(limit.clamp(1, 1000))
                .collect();
            return Ok((page, total));
        }
        Ok((metas, total))
    }

    pub fn list(&self) -> Result<HashMap<Uuid, UploadMeta>> {
        let (page, _) = self.list_page(1000, 0)?;
        Ok(page.into_iter().map(|m| (m.upload_id, m)).collect())
    }

    pub fn count(&self) -> usize {
        self.counts().0
    }

    /// (total uploads, completed uploads)
    pub fn counts(&self) -> (usize, usize) {
        self.db.upload_counts().unwrap_or((0, 0))
    }
}

fn meta_to_row(meta: &UploadMeta) -> UploadRow {
    UploadRow {
        upload_id: meta.upload_id.to_string(),
        case_id: meta.case_id.to_string(),
        artefact: match meta.artefact {
            ArtefactKind::AndroidBugreport => "android_bugreport".into(),
            ArtefactKind::IosSysdiagnose => "ios_sysdiagnose".into(),
            ArtefactKind::MessageReport => "message_report".into(),
        },
        total_size: meta.total_size,
        chunk_size: meta.chunk_size,
        chunk_count: meta.chunk_count,
        ciphertext_sha256: meta.ciphertext_sha256.clone(),
        received: meta.received.clone(),
        complete: meta.complete,
        created_at: meta.created_at.clone(),
        completed_at: meta.completed_at.clone(),
    }
}

fn row_to_meta(row: UploadRow) -> Result<UploadMeta> {
    let artefact = match row.artefact.as_str() {
        "ios_sysdiagnose" => ArtefactKind::IosSysdiagnose,
        "message_report" => ArtefactKind::MessageReport,
        _ => ArtefactKind::AndroidBugreport,
    };
    Ok(UploadMeta {
        upload_id: Uuid::parse_str(&row.upload_id)?,
        case_id: Uuid::parse_str(&row.case_id).unwrap_or(Uuid::nil()),
        artefact,
        total_size: row.total_size,
        chunk_size: row.chunk_size,
        chunk_count: row.chunk_count,
        ciphertext_sha256: row.ciphertext_sha256,
        received: row.received,
        complete: row.complete,
        created_at: row.created_at,
        completed_at: row.completed_at,
    })
}

fn walk_upload_metas(dir: &Path, out: &mut Vec<UploadMeta>) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let meta_path = path.join("meta.json");
        if meta_path.is_file() {
            if let Ok(bytes) = fs::read(&meta_path) {
                if let Ok(mut meta) = serde_json::from_slice::<UploadMeta>(&bytes) {
                    if meta.created_at.is_empty() {
                        if let Ok(m) = entry.metadata() {
                            if let Ok(t) = m.created().or_else(|_| m.modified()) {
                                meta.created_at = system_time_secs_str(t);
                            }
                        }
                    }
                    out.push(meta);
                    continue;
                }
            }
        }
        walk_upload_metas(&path, out)?;
    }
    Ok(())
}

fn unix_secs_str() -> String {
    system_time_secs_str(std::time::SystemTime::now())
}

fn system_time_secs_str(t: std::time::SystemTime) -> String {
    t.duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_default()
}
