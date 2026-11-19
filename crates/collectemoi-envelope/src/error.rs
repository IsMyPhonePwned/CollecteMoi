use thiserror::Error;

#[derive(Debug, Error)]
pub enum EnvelopeError {
    #[error("invalid magic (expected CM01)")]
    BadMagic,

    #[error("unsupported envelope version: {0}")]
    UnsupportedVersion(u8),

    #[error("truncated or malformed envelope")]
    Truncated,

    #[error("header decode failed: {0}")]
    HeaderDecode(String),

    #[error("header encode failed: {0}")]
    HeaderEncode(String),

    #[error("HPKE seal failed")]
    HpkeSeal,

    #[error("HPKE open failed")]
    HpkeOpen,

    #[error("STREAM encrypt failed at segment {0}")]
    StreamEncrypt(u32),

    #[error("STREAM decrypt failed at segment {0}")]
    StreamDecrypt(u32),

    #[error("plaintext SHA-256 mismatch")]
    PlaintextHashMismatch,

    #[error("original size mismatch")]
    SizeMismatch,

    #[error("device signature invalid")]
    BadSignature,

    #[error("invalid key material: {0}")]
    BadKey(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, EnvelopeError>;
