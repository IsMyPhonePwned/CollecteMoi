//! CollecteMoi artefact encryption envelope.
//!
//! Each artefact is encrypted to a **case** X25519 public key with HPKE
//! (RFC 9180: DHKEM-X25519 / HKDF-SHA256 / ChaCha20-Poly1305). The payload
//! uses an age-style STREAM of 64 KiB ChaCha20-Poly1305 segments. The header
//! is signed by the enrolled **device** P-256 key.
//!
//! Wire layout:
//! ```text
//! magic "CM01" (4) || header_len u32be (4) || header CBOR || sig 64 || STREAM ct
//! ```

mod envelope;
mod error;
pub mod ffi;
mod header;
mod hpke_wrap;
mod sign;
mod stream;

pub use envelope::{
    envelope_sha256, open, parse_and_verify, seal, seal_with_rng, MAGIC, SIGNATURE_LEN,
};
pub use error::{EnvelopeError, Result};
pub use header::{ArtefactKind, EnvelopeHeader};
pub use hpke_wrap::{CaseKeyPair, HPKE_INFO};
pub use sign::DeviceKeyPair;
pub use stream::{SEGMENT_SIZE, STREAM_NONCE_PREFIX_LEN, TAG_SIZE};
