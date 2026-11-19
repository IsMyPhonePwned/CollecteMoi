//! Age-style STREAM: 64 KiB ChaCha20-Poly1305 segments with a counter nonce
//! and a last-segment flag in the high bit of the counter.

use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    ChaCha20Poly1305, Key, Nonce,
};

use crate::error::{EnvelopeError, Result};

pub const SEGMENT_SIZE: usize = 64 * 1024;
pub const TAG_SIZE: usize = 16;
pub const STREAM_NONCE_PREFIX_LEN: usize = 8;
const NONCE_LEN: usize = 12;

/// Encrypt `plaintext` into age-style STREAM ciphertext.
pub fn seal(file_key: &[u8; 32], nonce_prefix: &[u8; 8], plaintext: &[u8]) -> Result<Vec<u8>> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(file_key));
    let mut out = Vec::with_capacity(plaintext.len() + TAG_SIZE * (plaintext.len() / SEGMENT_SIZE + 1));
    let mut offset = 0u64;
    let mut counter: u32 = 0;

    loop {
        let start = offset as usize;
        let remaining = plaintext.len().saturating_sub(start);
        let take = remaining.min(SEGMENT_SIZE);
        let last = take < SEGMENT_SIZE;
        let segment = &plaintext[start..start + take];

        let nonce = make_nonce(nonce_prefix, counter, last);
        let ct = cipher
            .encrypt(Nonce::from_slice(&nonce), Payload { msg: segment, aad: b"" })
            .map_err(|_| EnvelopeError::StreamEncrypt(counter))?;
        out.extend_from_slice(&ct);

        if last {
            break;
        }
        offset += take as u64;
        counter = counter
            .checked_add(1)
            .ok_or(EnvelopeError::StreamEncrypt(u32::MAX))?;
    }

    Ok(out)
}

/// Decrypt age-style STREAM ciphertext.
pub fn open(file_key: &[u8; 32], nonce_prefix: &[u8; 8], ciphertext: &[u8]) -> Result<Vec<u8>> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(file_key));
    let mut out = Vec::new();
    let mut offset = 0usize;
    let mut counter: u32 = 0;

    loop {
        if offset >= ciphertext.len() {
            return Err(EnvelopeError::Truncated);
        }

        let remaining = ciphertext.len() - offset;
        // A non-final segment is always SEGMENT_SIZE + TAG_SIZE.
        // The final segment may be shorter (including empty plaintext → TAG_SIZE only).
        let full = SEGMENT_SIZE + TAG_SIZE;
        let last = remaining <= full;
        let take = if last { remaining } else { full };
        if take < TAG_SIZE {
            return Err(EnvelopeError::Truncated);
        }

        let chunk = &ciphertext[offset..offset + take];
        let nonce = make_nonce(nonce_prefix, counter, last);
        let pt = cipher
            .decrypt(Nonce::from_slice(&nonce), Payload { msg: chunk, aad: b"" })
            .map_err(|_| EnvelopeError::StreamDecrypt(counter))?;
        out.extend_from_slice(&pt);
        offset += take;

        if last {
            break;
        }
        counter = counter
            .checked_add(1)
            .ok_or(EnvelopeError::StreamDecrypt(u32::MAX))?;
    }

    Ok(out)
}

fn make_nonce(prefix: &[u8; 8], counter: u32, last: bool) -> [u8; NONCE_LEN] {
    let mut nonce = [0u8; NONCE_LEN];
    nonce[..STREAM_NONCE_PREFIX_LEN].copy_from_slice(prefix);
    let mut c = counter;
    if last {
        c |= 1 << 31;
    }
    nonce[8..].copy_from_slice(&c.to_be_bytes());
    nonce
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::RngCore;

    fn roundtrip(len: usize) {
        let mut key = [0u8; 32];
        let mut nonce = [0u8; 8];
        rand::thread_rng().fill_bytes(&mut key);
        rand::thread_rng().fill_bytes(&mut nonce);
        let mut pt = vec![0u8; len];
        rand::thread_rng().fill_bytes(&mut pt);
        let ct = seal(&key, &nonce, &pt).unwrap();
        let out = open(&key, &nonce, &ct).unwrap();
        assert_eq!(out, pt);
    }

    #[test]
    fn empty() {
        roundtrip(0);
    }

    #[test]
    fn small() {
        roundtrip(17);
    }

    #[test]
    fn exact_segment() {
        roundtrip(SEGMENT_SIZE);
    }

    #[test]
    fn multi_segment() {
        roundtrip(SEGMENT_SIZE * 2 + 100);
    }

    #[test]
    fn tamper_detected() {
        let key = [7u8; 32];
        let nonce = [9u8; 8];
        let pt = b"hello stream";
        let mut ct = seal(&key, &nonce, pt).unwrap();
        ct[0] ^= 0xff;
        assert!(open(&key, &nonce, &ct).is_err());
    }
}
