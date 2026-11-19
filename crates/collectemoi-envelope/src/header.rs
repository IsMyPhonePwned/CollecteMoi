use serde::{Deserialize, Serialize};

/// Artefact kinds the envelope can carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtefactKind {
    AndroidBugreport,
    IosSysdiagnose,
    MessageReport,
}

impl ArtefactKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AndroidBugreport => "android_bugreport",
            Self::IosSysdiagnose => "ios_sysdiagnose",
            Self::MessageReport => "message_report",
        }
    }
}

/// Unsigned header fields sealed into the envelope (CBOR).
///
/// The device signs `magic || header_len || header_bytes`. The HPKE-wrapped
/// file key is bound to the artefact metadata via HPKE AAD.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvelopeHeader {
    /// Format version; currently always 1.
    pub v: u8,
    pub artefact: ArtefactKind,
    pub size: u64,
    #[serde(with = "serde_bytes")]
    pub sha256: Vec<u8>,
    /// HPKE encapsulated key (`enc`).
    #[serde(with = "serde_bytes")]
    pub hpke_enc: Vec<u8>,
    /// HPKE ciphertext wrapping the 32-byte file key.
    #[serde(with = "serde_bytes")]
    pub hpke_ct: Vec<u8>,
    /// 8-byte STREAM nonce prefix (counter fills the remaining 4 bytes).
    #[serde(with = "serde_bytes")]
    pub stream_nonce: Vec<u8>,
}

impl EnvelopeHeader {
    pub const VERSION: u8 = 1;

    pub fn encode(&self) -> Result<Vec<u8>, String> {
        let mut buf = Vec::new();
        ciborium::into_writer(self, &mut buf).map_err(|e| e.to_string())?;
        Ok(buf)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        ciborium::from_reader(bytes).map_err(|e| e.to_string())
    }
}
