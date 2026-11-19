use rand_core::{CryptoRng, RngCore};
use sha2::{Digest, Sha256};

use crate::error::{EnvelopeError, Result};
use crate::header::{ArtefactKind, EnvelopeHeader};
use crate::hpke_wrap::{self, CaseKeyPair};
use crate::sign::{self, DeviceKeyPair};
use crate::stream::{self, STREAM_NONCE_PREFIX_LEN};

/// Wire magic: CollecteMoi envelope v1.
pub const MAGIC: &[u8; 4] = b"CM01";
pub const SIGNATURE_LEN: usize = 64;

/// Seal plaintext into a CollecteMoi envelope.
///
/// Layout: `magic (4) || header_len u32be (4) || header || sig (64) || STREAM ciphertext`
pub fn seal(
    plaintext: &[u8],
    artefact: ArtefactKind,
    case_pk_bytes: &[u8],
    device: &DeviceKeyPair,
) -> Result<Vec<u8>> {
    seal_with_rng(
        plaintext,
        artefact,
        case_pk_bytes,
        device,
        &mut rand::thread_rng(),
    )
}

/// Like [`seal`], but takes an RNG so test vectors can be deterministic.
pub fn seal_with_rng<R: RngCore + CryptoRng>(
    plaintext: &[u8],
    artefact: ArtefactKind,
    case_pk_bytes: &[u8],
    device: &DeviceKeyPair,
    rng: &mut R,
) -> Result<Vec<u8>> {
    let case_pk = CaseKeyPair::public_only(case_pk_bytes)?;

    let mut hasher = Sha256::new();
    hasher.update(plaintext);
    let digest = hasher.finalize();
    let mut sha256 = [0u8; 32];
    sha256.copy_from_slice(&digest);

    let mut file_key = [0u8; 32];
    let mut stream_nonce = [0u8; STREAM_NONCE_PREFIX_LEN];
    rng.fill_bytes(&mut file_key);
    rng.fill_bytes(&mut stream_nonce);

    let aad = hpke_wrap::wrap_aad(artefact, plaintext.len() as u64, &sha256);
    let (hpke_enc, hpke_ct) = hpke_wrap::seal_file_key_rng(&case_pk, &file_key, &aad, rng)?;

    let header = EnvelopeHeader {
        v: EnvelopeHeader::VERSION,
        artefact,
        size: plaintext.len() as u64,
        sha256: sha256.to_vec(),
        hpke_enc,
        hpke_ct,
        stream_nonce: stream_nonce.to_vec(),
    };
    let header_bytes = header
        .encode()
        .map_err(EnvelopeError::HeaderEncode)?;

    let mut signed = Vec::with_capacity(8 + header_bytes.len());
    signed.extend_from_slice(MAGIC);
    signed.extend_from_slice(&(header_bytes.len() as u32).to_be_bytes());
    signed.extend_from_slice(&header_bytes);
    let signature = device.sign(&signed);

    let ciphertext = stream::seal(&file_key, &stream_nonce, plaintext)?;

    let mut out = signed;
    out.extend_from_slice(&signature);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Open an envelope, verifying the device signature and plaintext hash.
pub fn open(
    envelope: &[u8],
    case: &CaseKeyPair,
    device_pk_uncompressed: &[u8],
) -> Result<(ArtefactKind, Vec<u8>)> {
    let (header, ciphertext) = parse_and_verify(envelope, device_pk_uncompressed)?;
    decrypt_body(&header, ciphertext, case)
}

/// Parse header + verify device signature without decrypting (server-side check).
pub fn parse_and_verify<'a>(
    envelope: &'a [u8],
    device_pk_uncompressed: &[u8],
) -> Result<(EnvelopeHeader, &'a [u8])> {
    if envelope.len() < 8 + SIGNATURE_LEN {
        return Err(EnvelopeError::Truncated);
    }
    if &envelope[0..4] != MAGIC {
        return Err(EnvelopeError::BadMagic);
    }
    let header_len = u32::from_be_bytes(envelope[4..8].try_into().unwrap()) as usize;
    let header_end = 8 + header_len;
    let sig_end = header_end + SIGNATURE_LEN;
    if envelope.len() < sig_end {
        return Err(EnvelopeError::Truncated);
    }

    let header_bytes = &envelope[8..header_end];
    let signature = &envelope[header_end..sig_end];
    let ciphertext = &envelope[sig_end..];

    let signed = &envelope[..header_end];
    sign::verify_device_signature(device_pk_uncompressed, signed, signature)?;

    let header = EnvelopeHeader::decode(header_bytes).map_err(EnvelopeError::HeaderDecode)?;
    if header.v != EnvelopeHeader::VERSION {
        return Err(EnvelopeError::UnsupportedVersion(header.v));
    }
    if header.sha256.len() != 32 || header.stream_nonce.len() != STREAM_NONCE_PREFIX_LEN {
        return Err(EnvelopeError::Truncated);
    }

    Ok((header, ciphertext))
}

fn decrypt_body(
    header: &EnvelopeHeader,
    ciphertext: &[u8],
    case: &CaseKeyPair,
) -> Result<(ArtefactKind, Vec<u8>)> {
    let mut sha256 = [0u8; 32];
    sha256.copy_from_slice(&header.sha256);
    let aad = hpke_wrap::wrap_aad(header.artefact, header.size, &sha256);
    let file_key = hpke_wrap::open_file_key(&case.private, &header.hpke_enc, &header.hpke_ct, &aad)?;

    let mut nonce = [0u8; STREAM_NONCE_PREFIX_LEN];
    nonce.copy_from_slice(&header.stream_nonce);
    let plaintext = stream::open(&file_key, &nonce, ciphertext)?;

    if plaintext.len() as u64 != header.size {
        return Err(EnvelopeError::SizeMismatch);
    }
    let mut hasher = Sha256::new();
    hasher.update(&plaintext);
    let digest = hasher.finalize();
    if digest.as_slice() != header.sha256 {
        return Err(EnvelopeError::PlaintextHashMismatch);
    }

    Ok((header.artefact, plaintext))
}

/// Ciphertext SHA-256 of the full envelope bytes (used by the upload protocol).
pub fn envelope_sha256(envelope: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(envelope);
    let d = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&d);
    out
}
