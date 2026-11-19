//! CollecteMoi decrypt worker.
//!
//! Claims pending jobs from SQLite (`data/collectemoi.db`), decrypts CM01
//! envelopes with the case private key, and stores plaintext as opaque blobs
//! under `data/blobs/` while registering metadata in SQLite.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::Parser;
use collectemoi_db::{parse_uuid_id, safe_filename, BlobRow, Db, JobRow};
use collectemoi_envelope::{open, ArtefactKind, CaseKeyPair};
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(
    name = "collectemoi-worker",
    version,
    about = "Decrypt completed uploads into opaque blobs (no parsers)"
)]
struct Args {
    /// Same data directory as collectemoi-api
    #[arg(long, default_value = "./data")]
    data_dir: PathBuf,

    /// Poll interval in seconds
    #[arg(long, default_value_t = 2)]
    poll_secs: u64,

    /// Max jobs processed in parallel
    #[arg(long, default_value_t = 4)]
    workers: usize,

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

    let db = Arc::new(Db::open(&args.data_dir)?);
    tracing::info!(db = %db.path().display(), "sqlite catalogue ready");

    let keys_dir = args.data_dir.join("keys");
    let case = Arc::new(load_case_keys(
        &keys_dir,
        args.case_pk.as_deref(),
        args.case_sk.as_deref(),
    )?);
    let workers = args.workers.max(1);
    let sem = Arc::new(Semaphore::new(workers));

    tracing::info!(
        data = %args.data_dir.display(),
        workers,
        "collectemoi-worker polling (blob store only — no parsers)"
    );

    loop {
        if let Err(e) = poll_once(db.clone(), &blobs_dir, case.clone(), sem.clone()).await {
            tracing::error!(error = %e, "poll cycle failed");
        }
        tokio::time::sleep(Duration::from_secs(args.poll_secs)).await;
    }
}

async fn poll_once(
    db: Arc<Db>,
    blobs_dir: &Path,
    case: Arc<CaseKeyPair>,
    sem: Arc<Semaphore>,
) -> Result<()> {
    let pending = db.job_list_pending(64)?;
    let mut handles = Vec::new();

    for job in pending {
        let claimed = match db.job_claim(&job.upload_id)? {
            Some(j) => j,
            None => continue,
        };

        let permit = sem.clone().acquire_owned().await?;
        let case = case.clone();
        let blobs_dir = blobs_dir.to_path_buf();
        let db = db.clone();
        handles.push(tokio::task::spawn_blocking(move || {
            let _permit = permit;
            tracing::info!(upload = %claimed.upload_id, artefact = %claimed.artefact, "processing job");
            match process_job(&claimed, &blobs_dir, &case, &db) {
                Ok(blob_id) => {
                    db.job_mark_done(&claimed.upload_id)?;
                    // Best-effort cleanup of legacy JSON job drops.
                    let jobs = blobs_dir
                        .parent()
                        .unwrap_or(Path::new("."))
                        .join("jobs");
                    for ext in ["json", "json.claimed", "json.done"] {
                        let _ = fs::remove_file(jobs.join(format!("{}.{}", claimed.upload_id, ext)));
                    }
                    tracing::info!(upload = %claimed.upload_id, blob = %blob_id, "blob stored");
                    Ok::<(), anyhow::Error>(())
                }
                Err(e) => {
                    tracing::error!(upload = %claimed.upload_id, error = %e, "job failed");
                    let _ = db.job_mark_failed(&claimed.upload_id, &e.to_string());
                    Err(e)
                }
            }
        }));
    }

    for h in handles {
        match h.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::warn!(error = %e, "job task error"),
            Err(e) => tracing::warn!(error = %e, "job task join error"),
        }
    }
    Ok(())
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
        content_type: "application/octet-stream".into(),
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
        "message_report" => "message_report.bin".into(),
        other => format!("{other}.bin"),
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
