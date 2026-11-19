use collectemoi_envelope::{
    envelope_sha256, open, parse_and_verify, seal, ArtefactKind, CaseKeyPair, DeviceKeyPair,
};

#[test]
fn seal_open_roundtrip() {
    let case = CaseKeyPair::generate();
    let device = DeviceKeyPair::generate();
    let plaintext = b"bugreport contents for CollecteMoi roundtrip test";

    let envelope = seal(
        plaintext,
        ArtefactKind::AndroidBugreport,
        &case.public_bytes(),
        &device,
    )
    .expect("seal");

    assert_eq!(&envelope[0..4], b"CM01");
    let _ = envelope_sha256(&envelope);

    let (header, _ct) =
        parse_and_verify(&envelope, &device.public_bytes()).expect("parse_and_verify");
    assert_eq!(header.artefact, ArtefactKind::AndroidBugreport);
    assert_eq!(header.size, plaintext.len() as u64);

    let (kind, out) = open(&envelope, &case, &device.public_bytes()).expect("open");
    assert_eq!(kind, ArtefactKind::AndroidBugreport);
    assert_eq!(out, plaintext);
}

#[test]
fn wrong_device_key_rejected() {
    let case = CaseKeyPair::generate();
    let device = DeviceKeyPair::generate();
    let other = DeviceKeyPair::generate();
    let envelope = seal(b"x", ArtefactKind::MessageReport, &case.public_bytes(), &device).unwrap();
    assert!(parse_and_verify(&envelope, &other.public_bytes()).is_err());
}

#[test]
fn wrong_case_key_rejected() {
    let case = CaseKeyPair::generate();
    let other = CaseKeyPair::generate();
    let device = DeviceKeyPair::generate();
    let envelope = seal(
        b"sysdiagnose",
        ArtefactKind::IosSysdiagnose,
        &case.public_bytes(),
        &device,
    )
    .unwrap();
    assert!(open(&envelope, &other, &device.public_bytes()).is_err());
}

#[test]
fn large_payload_roundtrip() {
    let case = CaseKeyPair::generate();
    let device = DeviceKeyPair::generate();
    // Cross a few STREAM segment boundaries (64 KiB).
    let plaintext = vec![0xA5u8; 64 * 1024 * 3 + 123];
    let envelope = seal(
        &plaintext,
        ArtefactKind::AndroidBugreport,
        &case.public_bytes(),
        &device,
    )
    .unwrap();
    let (_, out) = open(&envelope, &case, &device.public_bytes()).unwrap();
    assert_eq!(out, plaintext);
}
