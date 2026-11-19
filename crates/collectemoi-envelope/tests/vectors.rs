//! Deterministic test vectors with a seeded RNG.
//!
//! Keys are persisted under `tests/vectors/` so they stay stable across runs.
//! Regenerate the golden envelope with:
//! `cargo test -p collectemoi-envelope regenerate_golden_envelope -- --ignored`

use std::path::PathBuf;

use collectemoi_envelope::{
    envelope_sha256, open, seal_with_rng, ArtefactKind, CaseKeyPair, DeviceKeyPair,
};
use rand::SeedableRng;
use rand_chacha::ChaCha20Rng;
use serde_json::json;

fn vector_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/vectors")
}

fn load_or_create_keys() -> (CaseKeyPair, DeviceKeyPair) {
    let dir = vector_dir();
    std::fs::create_dir_all(&dir).unwrap();

    let case = if dir.join("case.pk").exists() {
        CaseKeyPair::from_parts(
            &std::fs::read(dir.join("case.pk")).unwrap(),
            &std::fs::read(dir.join("case.sk")).unwrap(),
        )
        .unwrap()
    } else {
        let kp = CaseKeyPair::generate();
        std::fs::write(dir.join("case.pk"), kp.public_bytes()).unwrap();
        std::fs::write(dir.join("case.sk"), kp.private_bytes()).unwrap();
        kp
    };

    let device = if dir.join("device.sk").exists() {
        let d = DeviceKeyPair::from_private_bytes(&std::fs::read(dir.join("device.sk")).unwrap())
            .unwrap();
        if !dir.join("device.pk").exists() {
            std::fs::write(dir.join("device.pk"), d.public_bytes()).unwrap();
        }
        d
    } else {
        let d = DeviceKeyPair::generate();
        std::fs::write(dir.join("device.pk"), d.public_bytes()).unwrap();
        std::fs::write(dir.join("device.sk"), d.private_bytes()).unwrap();
        d
    };

    (case, device)
}

#[test]
fn deterministic_golden_envelope() {
    let (case, device) = load_or_create_keys();
    let plaintext = b"CollecteMoi vector plaintext v1";
    let mut rng = ChaCha20Rng::from_seed([42u8; 32]);

    let envelope = seal_with_rng(
        plaintext,
        ArtefactKind::AndroidBugreport,
        &case.public_bytes(),
        &device,
        &mut rng,
    )
    .unwrap();

    let (_, out) = open(&envelope, &case, &device.public_bytes()).unwrap();
    assert_eq!(out, plaintext);

    let digest = hex::encode(envelope_sha256(&envelope));
    let golden_path = vector_dir().join("envelope.sha256");

    if !golden_path.exists() {
        std::fs::write(vector_dir().join("envelope.bin"), &envelope).unwrap();
        std::fs::write(&golden_path, format!("{digest}\n")).unwrap();
        std::fs::write(
            vector_dir().join("meta.json"),
            serde_json::to_vec_pretty(&json!({
                "artefact": "android_bugreport",
                "plaintext": String::from_utf8_lossy(plaintext),
                "plaintext_len": plaintext.len(),
                "envelope_sha256": digest,
                "envelope_len": envelope.len(),
                "rng_seed": "0x2a * 32",
            }))
            .unwrap(),
        )
        .unwrap();
        return;
    }

    let expected = std::fs::read_to_string(&golden_path)
        .unwrap()
        .trim()
        .to_string();
    assert_eq!(digest, expected, "envelope SHA-256 drifted from golden vector");
    let golden_bytes = std::fs::read(vector_dir().join("envelope.bin")).unwrap();
    assert_eq!(envelope, golden_bytes, "envelope bytes drifted from golden vector");
}

/// Re-run with: `cargo test -p collectemoi-envelope regenerate_golden_envelope -- --ignored --nocapture`
#[test]
#[ignore = "destructive: rewrite tests/vectors golden files"]
fn regenerate_golden_envelope() {
    let dir = vector_dir();
    let _ = std::fs::remove_file(dir.join("envelope.sha256"));
    let _ = std::fs::remove_file(dir.join("envelope.bin"));
    let _ = std::fs::remove_file(dir.join("meta.json"));
    deterministic_golden_envelope();
}
