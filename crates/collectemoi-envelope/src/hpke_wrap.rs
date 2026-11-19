//! HPKE file-key wrap: DHKEM-X25519 / HKDF-SHA256 / ChaCha20-Poly1305 (RFC 9180).

use hpke::{
    aead::ChaCha20Poly1305,
    kdf::HkdfSha256,
    kem::X25519HkdfSha256,
    Deserializable, Kem as KemTrait, OpModeR, OpModeS, Serializable,
};
use rand_core::OsRng;

use crate::error::{EnvelopeError, Result};
use crate::header::ArtefactKind;

pub type Kem = X25519HkdfSha256;
pub type Aead = ChaCha20Poly1305;
pub type Kdf = HkdfSha256;

pub const HPKE_INFO: &[u8] = b"collectemoi-envelope-v1";

/// Case (recipient) X25519 key pair.
#[derive(Clone)]
pub struct CaseKeyPair {
    pub public: <Kem as KemTrait>::PublicKey,
    pub private: <Kem as KemTrait>::PrivateKey,
}

impl CaseKeyPair {
    pub fn generate() -> Self {
        let (private, public) = Kem::gen_keypair(&mut OsRng);
        Self { public, private }
    }

    pub fn public_bytes(&self) -> Vec<u8> {
        self.public.to_bytes().to_vec()
    }

    pub fn private_bytes(&self) -> Vec<u8> {
        self.private.to_bytes().to_vec()
    }

    pub fn from_parts(public: &[u8], private: &[u8]) -> Result<Self> {
        let public = <Kem as KemTrait>::PublicKey::from_bytes(public)
            .map_err(|e| EnvelopeError::BadKey(format!("case public: {e:?}")))?;
        let private = <Kem as KemTrait>::PrivateKey::from_bytes(private)
            .map_err(|e| EnvelopeError::BadKey(format!("case private: {e:?}")))?;
        Ok(Self { public, private })
    }

    pub fn public_only(public: &[u8]) -> Result<<Kem as KemTrait>::PublicKey> {
        <Kem as KemTrait>::PublicKey::from_bytes(public)
            .map_err(|e| EnvelopeError::BadKey(format!("case public: {e:?}")))
    }
}

/// Build HPKE AAD that binds the wrapped key to artefact metadata.
pub fn wrap_aad(artefact: ArtefactKind, size: u64, sha256: &[u8; 32]) -> Vec<u8> {
    let mut aad = Vec::with_capacity(32 + 1 + 8 + 32);
    aad.extend_from_slice(artefact.as_str().as_bytes());
    aad.push(0);
    aad.extend_from_slice(&size.to_le_bytes());
    aad.extend_from_slice(sha256);
    aad
}

pub fn seal_file_key_rng<R: rand_core::CryptoRng + rand_core::RngCore>(
    recipient_pk: &<Kem as KemTrait>::PublicKey,
    file_key: &[u8; 32],
    aad: &[u8],
    rng: &mut R,
) -> Result<(Vec<u8>, Vec<u8>)> {
    let (encapped, mut ctx) =
        hpke::setup_sender::<Aead, Kdf, Kem, _>(&OpModeS::Base, recipient_pk, HPKE_INFO, rng)
            .map_err(|_| EnvelopeError::HpkeSeal)?;
    let ct = ctx
        .seal(file_key, aad)
        .map_err(|_| EnvelopeError::HpkeSeal)?;
    Ok((encapped.to_bytes().to_vec(), ct))
}

pub fn open_file_key(
    recipient_sk: &<Kem as KemTrait>::PrivateKey,
    enc: &[u8],
    ct: &[u8],
    aad: &[u8],
) -> Result<[u8; 32]> {
    let encapped = <Kem as KemTrait>::EncappedKey::from_bytes(enc)
        .map_err(|_| EnvelopeError::HpkeOpen)?;
    let mut ctx =
        hpke::setup_receiver::<Aead, Kdf, Kem>(&OpModeR::Base, recipient_sk, &encapped, HPKE_INFO)
            .map_err(|_| EnvelopeError::HpkeOpen)?;
    let pt = ctx.open(ct, aad).map_err(|_| EnvelopeError::HpkeOpen)?;
    if pt.len() != 32 {
        return Err(EnvelopeError::HpkeOpen);
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&pt);
    Ok(key)
}
