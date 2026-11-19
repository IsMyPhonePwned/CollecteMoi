//! Device attestation signature: P-256 ECDSA over SHA-256.

use p256::ecdsa::{
    signature::{Signer, Verifier},
    Signature, SigningKey, VerifyingKey,
};
use rand_core::OsRng;

use crate::error::{EnvelopeError, Result};

/// Hardware-backed in production; software key for tests and the CLI.
#[derive(Clone)]
pub struct DeviceKeyPair {
    signing: SigningKey,
}

impl DeviceKeyPair {
    pub fn generate() -> Self {
        Self {
            signing: SigningKey::random(&mut OsRng),
        }
    }

    pub fn from_private_bytes(bytes: &[u8]) -> Result<Self> {
        let signing = SigningKey::from_slice(bytes)
            .map_err(|e| EnvelopeError::BadKey(format!("device private: {e}")))?;
        Ok(Self { signing })
    }

    pub fn private_bytes(&self) -> Vec<u8> {
        self.signing.to_bytes().to_vec()
    }

    pub fn public_bytes(&self) -> Vec<u8> {
        self.signing
            .verifying_key()
            .to_encoded_point(false)
            .as_bytes()
            .to_vec()
    }

    pub fn verifying_key(&self) -> VerifyingKey {
        *self.signing.verifying_key()
    }

    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        let sig: Signature = self.signing.sign(message);
        let bytes = sig.to_bytes();
        let mut out = [0u8; 64];
        out.copy_from_slice(&bytes);
        out
    }
}

pub fn verify_device_signature(public_uncompressed: &[u8], message: &[u8], sig: &[u8]) -> Result<()> {
    if sig.len() != 64 {
        return Err(EnvelopeError::BadSignature);
    }
    let vk = VerifyingKey::from_sec1_bytes(public_uncompressed)
        .map_err(|e| EnvelopeError::BadKey(format!("device public: {e}")))?;
    let signature = Signature::from_slice(sig).map_err(|_| EnvelopeError::BadSignature)?;
    vk.verify(message, &signature)
        .map_err(|_| EnvelopeError::BadSignature)
}
