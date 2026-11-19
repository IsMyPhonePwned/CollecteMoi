//! CLI for envelope encrypt/decrypt and chunked upload against collectemoi-api.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use collectemoi_envelope::{
    envelope_sha256, open, seal, ArtefactKind, CaseKeyPair, DeviceKeyPair,
};
use collectemoi_proto::{
    ArtefactKind as ProtoKind, CompleteUploadRequest, CreateUploadRequest, CreateUploadResponse,
    UploadStatus, DEFAULT_CHUNK_SIZE,
};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Parser)]
#[command(name = "collectemoi", about = "CollecteMoi envelope + upload CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate a case X25519 key pair (recipient).
    GenCaseKey {
        #[arg(long, default_value = "case")]
        out: PathBuf,
    },
    /// Generate a device P-256 key pair (signer).
    GenDeviceKey {
        #[arg(long, default_value = "device")]
        out: PathBuf,
    },
    /// Encrypt a file into a CollecteMoi envelope.
    Seal {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        case_pk: PathBuf,
        #[arg(long)]
        device_sk: PathBuf,
        #[arg(long, value_enum, default_value = "android-bugreport")]
        artefact: ArtefactArg,
    },
    /// Decrypt an envelope (requires case private key).
    Open {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        case_sk: PathBuf,
        #[arg(long)]
        case_pk: PathBuf,
        #[arg(long)]
        device_pk: PathBuf,
    },
    /// Encrypt and upload a file via the chunked protocol.
    Upload {
        #[arg(long)]
        input: PathBuf,
        #[arg(long, default_value = "https://127.0.0.1:8080")]
        server: String,
        #[arg(long)]
        case_id: Uuid,
        #[arg(long)]
        case_pk: PathBuf,
        #[arg(long)]
        device_sk: PathBuf,
        #[arg(long, value_enum, default_value = "android-bugreport")]
        artefact: ArtefactArg,
        #[arg(long, default_value_t = DEFAULT_CHUNK_SIZE)]
        chunk_size: u64,
        /// Accept self-signed / untrusted TLS certificates (lab default for auto-generated certs).
        #[arg(long, default_value_t = true)]
        insecure: bool,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum ArtefactArg {
    AndroidBugreport,
    IosSysdiagnose,
    MessageReport,
}

impl ArtefactArg {
    fn to_env(self) -> ArtefactKind {
        match self {
            Self::AndroidBugreport => ArtefactKind::AndroidBugreport,
            Self::IosSysdiagnose => ArtefactKind::IosSysdiagnose,
            Self::MessageReport => ArtefactKind::MessageReport,
        }
    }

    fn to_proto(self) -> ProtoKind {
        match self {
            Self::AndroidBugreport => ProtoKind::AndroidBugreport,
            Self::IosSysdiagnose => ProtoKind::IosSysdiagnose,
            Self::MessageReport => ProtoKind::MessageReport,
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::GenCaseKey { out } => {
            let kp = CaseKeyPair::generate();
            write_bin(out.with_extension("pk"), &kp.public_bytes())?;
            write_bin(out.with_extension("sk"), &kp.private_bytes())?;
            println!("wrote {}.pk / {}.sk", out.display(), out.display());
        }
        Commands::GenDeviceKey { out } => {
            let kp = DeviceKeyPair::generate();
            write_bin(out.with_extension("pk"), &kp.public_bytes())?;
            write_bin(out.with_extension("sk"), &kp.private_bytes())?;
            println!("wrote {}.pk / {}.sk", out.display(), out.display());
        }
        Commands::Seal {
            input,
            output,
            case_pk,
            device_sk,
            artefact,
        } => {
            let pt = fs::read(&input)?;
            let case_pk = fs::read(&case_pk)?;
            let device = DeviceKeyPair::from_private_bytes(&fs::read(&device_sk)?)?;
            let env = seal(&pt, artefact.to_env(), &case_pk, &device)?;
            fs::write(&output, &env)?;
            println!(
                "sealed {} bytes → {} (sha256 {})",
                pt.len(),
                output.display(),
                hex::encode(envelope_sha256(&env))
            );
        }
        Commands::Open {
            input,
            output,
            case_sk,
            case_pk,
            device_pk,
        } => {
            let env = fs::read(&input)?;
            let case = CaseKeyPair::from_parts(&fs::read(&case_pk)?, &fs::read(&case_sk)?)?;
            let device_pk = fs::read(&device_pk)?;
            let (kind, pt) = open(&env, &case, &device_pk)?;
            fs::write(&output, &pt)?;
            println!("opened {:?} → {} ({} bytes)", kind, output.display(), pt.len());
        }
        Commands::Upload {
            input,
            server,
            case_id,
            case_pk,
            device_sk,
            artefact,
            chunk_size,
            insecure,
        } => {
            upload(
                &input,
                &server,
                case_id,
                &case_pk,
                &device_sk,
                artefact,
                chunk_size,
                insecure,
            )?;
        }
    }
    Ok(())
}

fn write_bin(path: PathBuf, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, bytes)?;
    Ok(())
}

fn upload(
    input: &Path,
    server: &str,
    case_id: Uuid,
    case_pk: &Path,
    device_sk: &Path,
    artefact: ArtefactArg,
    chunk_size: u64,
    insecure: bool,
) -> Result<()> {
    let pt = fs::read(input).with_context(|| format!("read {}", input.display()))?;
    let case_pk_bytes = fs::read(case_pk)?;
    let device = DeviceKeyPair::from_private_bytes(&fs::read(device_sk)?)?;
    let device_pk_hex = hex::encode(device.public_bytes());

    println!("encrypting {} bytes…", pt.len());
    let envelope = seal(&pt, artefact.to_env(), &case_pk_bytes, &device)?;
    let total_size = envelope.len() as u64;
    let ciphertext_sha256 = hex::encode(envelope_sha256(&envelope));
    let chunk_count = total_size.div_ceil(chunk_size).max(1) as u32;

    let client = reqwest::blocking::Client::builder()
        .danger_accept_invalid_certs(insecure)
        .build()
        .context("build HTTP client")?;
    let create = CreateUploadRequest {
        case_id,
        artefact: artefact.to_proto(),
        total_size,
        chunk_size,
        chunk_count,
        ciphertext_sha256: ciphertext_sha256.clone(),
    };
    let resp = client
        .post(format!("{server}/v1/uploads"))
        .json(&create)
        .send()
        .context("POST /v1/uploads")?;
    if !resp.status().is_success() {
        bail!("create upload failed: {} {}", resp.status(), resp.text()?);
    }
    let created: CreateUploadResponse = resp.json()?;
    let upload_id = created.upload_id;
    println!("upload_id={upload_id} chunks={chunk_count} size={total_size}");

    for n in 0..chunk_count {
        let start = (n as u64 * chunk_size) as usize;
        let end = ((n as u64 + 1) * chunk_size).min(total_size) as usize;
        let chunk = &envelope[start..end];
        let mut hasher = Sha256::new();
        hasher.update(chunk);
        let digest = format!("sha-256={}", hex::encode(hasher.finalize()));

        let resp = client
            .put(format!("{server}/v1/uploads/{upload_id}/chunks/{n}"))
            .header("Digest", &digest)
            .body(chunk.to_vec())
            .send()
            .with_context(|| format!("PUT chunk {n}"))?;
        if !resp.status().is_success() {
            bail!("chunk {n} failed: {} {}", resp.status(), resp.text()?);
        }
        print!(".");
        let _ = std::io::Write::flush(&mut std::io::stdout());
    }
    println!();

    let status: UploadStatus = client
        .get(format!("{server}/v1/uploads/{upload_id}"))
        .send()?
        .json()?;
    if status.received.iter().any(|r| !*r) {
        bail!("server missing chunks after upload: {:?}", status.received);
    }

    let complete = CompleteUploadRequest {
        device_public_key: device_pk_hex,
    };
    let resp = client
        .post(format!("{server}/v1/uploads/{upload_id}/complete"))
        .json(&complete)
        .send()
        .context("POST complete")?;
    if !resp.status().is_success() {
        bail!("complete failed: {} {}", resp.status(), resp.text()?);
    }
    println!("complete: {}", resp.text()?);
    Ok(())
}
