//! CollecteMoi decrypt worker.
//!
//! Claims pending jobs from SQLite (`data/collectemoi.db`), decrypts CM01
//! envelopes with the case private key, and stores plaintext as opaque blobs
//! under `data/blobs/` while registering metadata in SQLite.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::Parser;
use collectemoi_db::{parse_uuid_id, safe_filename, BlobRow, Db, JobRow};
use collectemoi_envelope::{open, ArtefactKind, CaseKeyPair};
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;
use tracing_subscriber::EnvFilter;

const VERSION: &str = {
    const BAKED: &str = env!("COLLECTEMOI_BUILD_VERSION");
    if BAKED.is_empty() {
        env!("CARGO_PKG_VERSION")
    } else {
        BAKED
    }
};

fn default_workers() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get().saturating_mul(2))
        .unwrap_or(8)
        .clamp(8, 32)
}

#[derive(Debug, Parser)]
#[command(
    name = "collectemoi-worker",
    version = VERSION,
    about = "Decrypt completed uploads into opaque blobs (no parsers)"
)]
struct Args {
    /// Same data directory as collectemoi-api
    #[arg(long, default_value = "./data")]
    data_dir: PathBuf,

    /// Idle poll interval in seconds (busy queue polls faster)
    #[arg(long, default_value_t = 1)]
    poll_secs: u64,

    /// Max jobs processed in parallel (`0` = auto from CPU count)
    #[arg(long, default_value_t = 0)]
    workers: usize,

    /// SQLite connection pool size (`0` = auto)
    #[arg(long, default_value_t = 0)]
    db_pool_size: usize,

    /// Optional case public key file (hex). Default: <data-dir>/keys/case.pk.hex
    #[arg(long)]
    case_pk: Option<PathBuf>,

    /// Optional case private key file (hex). Default: <data-dir>/keys/case.sk.hex
    #[arg(long)]
    case_sk: Option<PathBuf>,

    /// Log filter
    #[arg(long, default_value = "collectemoi_worker=info")]
    log: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(&args.log))
        .init();

    let blobs_dir = args.data_dir.join("blobs");
    fs::create_dir_all(&blobs_dir)?;
    fs::create_dir_all(args.data_dir.join("jobs"))?;

    let db = Arc::new(Db::open_with_pool_size(&args.data_dir, args.db_pool_size)?);
    tracing::info!(
        db = %db.path().display(),
        pool = db.pool_size(),
        "sqlite catalogue ready"
    );

    let keys_dir = args.data_dir.join("keys");
    let case = Arc::new(load_case_keys(
        &keys_dir,
        args.case_pk.as_deref(),
        args.case_sk.as_deref(),
    )?);
    let workers = if args.workers == 0 {
        default_workers()
    } else {
        args.workers.max(1)
    };
    let sem = Arc::new(Semaphore::new(workers));
    let in_flight = Arc::new(AtomicUsize::new(0));

    tracing::info!(
        data = %args.data_dir.display(),
        workers,
        "collectemoi-worker polling (blob store only — no parsers)"
    );

    loop {
        let idle = match fill_slots(
            db.clone(),
            &blobs_dir,
            case.clone(),
            sem.clone(),
            in_flight.clone(),
            workers,
        )
        .await
        {
            Ok(n) => n == 0,
            Err(e) => {
                tracing::error!(error = %e, "poll cycle failed");
                true
            }
        };
        let delay = if idle {
            Duration::from_secs(args.poll_secs.max(1))
        } else {
            // Keep claiming while the queue is hot; brief yield for fair scheduling.
            Duration::from_millis(50)
        };
        tokio::time::sleep(delay).await;
    }
}

/// Claim and spawn up to free worker slots. Returns how many jobs were started.
async fn fill_slots(
    db: Arc<Db>,
    blobs_dir: &Path,
    case: Arc<CaseKeyPair>,
    sem: Arc<Semaphore>,
    in_flight: Arc<AtomicUsize>,
    workers: usize,
) -> Result<usize> {
    let free = workers.saturating_sub(in_flight.load(Ordering::SeqCst));
    if free == 0 {
        return Ok(0);
    }

    let claimed = {
        let db = db.clone();
        tokio::task::spawn_blocking(move || db.job_claim_batch(free))
            .await
            .context("claim batch join")??
    };
    if claimed.is_empty() {
        return Ok(0);
    }

    let started = claimed.len();
    for job in claimed {
        let permit = sem.clone().acquire_owned().await?;
        in_flight.fetch_add(1, Ordering::SeqCst);
        let case = case.clone();
        let blobs_dir = blobs_dir.to_path_buf();
        let db = db.clone();
        let in_flight = in_flight.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            tracing::info!(upload = %job.upload_id, artefact = %job.artefact, "processing job");
            let result = (|| -> Result<()> {
                match process_job(&job, &blobs_dir, &case, &db) {
                    Ok(blob_id) => {
                        db.job_mark_done(&job.upload_id)?;
                        let jobs = blobs_dir
                            .parent()
                            .unwrap_or(Path::new("."))
                            .join("jobs");
                        for ext in ["json", "json.claimed", "json.done"] {
                            let _ =
                                fs::remove_file(jobs.join(format!("{}.{}", job.upload_id, ext)));
                        }
                        tracing::info!(upload = %job.upload_id, blob = %blob_id, "blob stored");
                        Ok(())
                    }
                    Err(e) => {
                        tracing::error!(upload = %job.upload_id, error = %e, "job failed");
                        let _ = db.job_mark_failed(&job.upload_id, &e.to_string());
                        Err(e)
                    }
                }
            })();
            in_flight.fetch_sub(1, Ordering::SeqCst);
            if let Err(e) = result {
                tracing::warn!(upload = %job.upload_id, error = %e, "job task error");
            }
        });
    }
    Ok(started)
}

fn process_job(job: &JobRow, blobs_dir: &Path, case: &CaseKeyPair, db: &Db) -> Result<String> {
    let upload_id = parse_uuid_id(&job.upload_id).context("job upload_id must be a UUID")?;

    // Envelope path must stay under data_dir/uploads (reject traversal via forged DB rows).
    let envelope_path = PathBuf::from(&job.envelope_path);
    let data_dir = blobs_dir.parent().unwrap_or(blobs_dir);
    let canon_data = fs::canonicalize(data_dir).unwrap_or_else(|_| data_dir.to_path_buf());
    let canon_env = fs::canonicalize(&envelope_path)
        .with_context(|| format!("read envelope {}", envelope_path.display()))?;
    if !canon_env.starts_with(&canon_data) {
        bail!("envelope_path escapes data dir");
    }

    let envelope = fs::read(&canon_env)
        .with_context(|| format!("read envelope {}", canon_env.display()))?;
    let device_pk = hex::decode(job.device_public_key.trim())?;

    let (kind, plaintext) = open(&envelope, case, &device_pk)?;
    let sha = {
        let mut h = Sha256::new();
        h.update(&plaintext);
        hex::encode(h.finalize())
    };

    let artefact = if job.artefact.is_empty() {
        artefact_name(kind).to_string()
    } else {
        job.artefact.clone()
    };

    let blob_id = upload_id.to_string();
    let dir = shard_dir(blobs_dir, &upload_id);
    fs::create_dir_all(&dir)?;

    let bin_path = dir.join("blob.bin");
    let tmp = dir.join("blob.bin.tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(&plaintext)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, &bin_path)?;

    let created_at = now_secs_str();
    let filename = safe_filename(&suggested_filename(&artefact, looks_like_zip(&plaintext)));
    let content_type = suggested_content_type(&artefact, &plaintext);
    let meta = BlobRow {
        blob_id: blob_id.clone(),
        upload_id: blob_id.clone(),
        case_id: job.case_id.clone(),
        artefact: artefact.clone(),
        size: plaintext.len() as u64,
        sha256: sha,
        created_at: created_at.clone(),
        queued_at: job.queued_at.clone(),
        device_public_key: job.device_public_key.trim().to_string(),
        filename: filename.clone(),
        content_type,
    };
    db.blob_upsert(&meta)?;

    // Mirror meta.json for operators browsing the tree.
    let meta_path = dir.join("meta.json");
    let meta_tmp = dir.join("meta.json.tmp");
    fs::write(&meta_tmp, serde_json::to_vec_pretty(&meta)?)?;
    fs::rename(&meta_tmp, &meta_path)?;

    let legacy = data_dir.join("analysis").join(&blob_id);
    let _ = fs::create_dir_all(&legacy);
    let _ = fs::write(
        legacy.join("BLOB_ONLY.txt"),
        "Parsers disabled. Plaintext is stored as an opaque blob under data/blobs/.\n",
    );
    let _ = fs::write(
        legacy.join("blob_ref.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "blob_id": blob_id,
            "path": meta_path,
        }))?,
    );

    Ok(blob_id)
}

fn shard_dir(root: &Path, id: &uuid::Uuid) -> PathBuf {
    let s = id.to_string();
    root.join(&s[0..2]).join(&s[2..4]).join(&s)
}

fn artefact_name(kind: ArtefactKind) -> &'static str {
    match kind {
        ArtefactKind::AndroidBugreport => "android_bugreport",
        ArtefactKind::IosSysdiagnose => "ios_sysdiagnose",
        ArtefactKind::MessageReport => "message_report",
    }
}

fn suggested_filename(artefact: &str, is_zip: bool) -> String {
    match artefact {
        "android_bugreport" if is_zip => "bugreport.zip".into(),
        "android_bugreport" => "bugreport.bin".into(),
        "ios_sysdiagnose" => "sysdiagnose.tar.gz".into(),
        "message_report" => "message_report.json".into(),
        other => format!("{other}.bin"),
    }
}

fn suggested_content_type(artefact: &str, plaintext: &[u8]) -> String {
    match artefact {
        "message_report" => "application/json; charset=utf-8".into(),
        "android_bugreport" if looks_like_zip(plaintext) => "application/zip".into(),
        "ios_sysdiagnose"
            if plaintext.len() >= 2 && plaintext[0] == 0x1f && plaintext[1] == 0x8b =>
        {
            "application/gzip".into()
        }
        _ => "application/octet-stream".into(),
    }
}

fn looks_like_zip(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes[0] == 0x50 && bytes[1] == 0x4b
}

fn now_secs_str() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_default()
}

fn load_case_keys(
    keys_dir: &Path,
    case_pk: Option<&Path>,
    case_sk: Option<&Path>,
) -> Result<CaseKeyPair> {
    let pk_path = case_pk
        .map(Path::to_path_buf)
        .unwrap_or_else(|| keys_dir.join("case.pk.hex"));
    let sk_path = case_sk
        .map(Path::to_path_buf)
        .unwrap_or_else(|| keys_dir.join("case.sk.hex"));

    let pk = fs::read_to_string(&pk_path)
        .with_context(|| {
            format!(
                "missing {} — start the API once to generate keys",
                pk_path.display()
            )
        })?
        .trim()
        .to_string();
    let sk = fs::read_to_string(&sk_path)
        .with_context(|| format!("missing {}", sk_path.display()))?
        .trim()
        .to_string();
    if pk.is_empty() || sk.is_empty() {
        bail!("empty case keys");
    }
    Ok(CaseKeyPair::from_parts(&hex::decode(pk)?, &hex::decode(sk)?)?)
}
