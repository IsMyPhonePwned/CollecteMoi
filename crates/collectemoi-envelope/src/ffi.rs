//! C ABI for Flutter / other embedders.
//!
//! All heap buffers returned via out-pointers must be freed with [`cm_free`].

use std::os::raw::{c_int, c_uchar};
use std::slice;

use crate::{seal, ArtefactKind, CaseKeyPair, DeviceKeyPair};

const OK: c_int = 0;
const ERR: c_int = 1;

#[no_mangle]
pub unsafe extern "C" fn cm_free(ptr: *mut c_uchar, len: usize) {
    if ptr.is_null() || len == 0 {
        return;
    }
    let _ = Vec::from_raw_parts(ptr, len, len);
}

#[no_mangle]
pub unsafe extern "C" fn cm_generate_case_keypair(
    out_pk: *mut *mut c_uchar,
    out_pk_len: *mut usize,
    out_sk: *mut *mut c_uchar,
    out_sk_len: *mut usize,
) -> c_int {
    let kp = CaseKeyPair::generate();
    write_buf(out_pk, out_pk_len, &kp.public_bytes());
    write_buf(out_sk, out_sk_len, &kp.private_bytes());
    OK
}

#[no_mangle]
pub unsafe extern "C" fn cm_generate_device_keypair(
    out_pk: *mut *mut c_uchar,
    out_pk_len: *mut usize,
    out_sk: *mut *mut c_uchar,
    out_sk_len: *mut usize,
) -> c_int {
    let kp = DeviceKeyPair::generate();
    write_buf(out_pk, out_pk_len, &kp.public_bytes());
    write_buf(out_sk, out_sk_len, &kp.private_bytes());
    OK
}

/// artefact: 0=android_bugreport, 1=ios_sysdiagnose, 2=message_report
#[no_mangle]
pub unsafe extern "C" fn cm_seal(
    plaintext: *const c_uchar,
    plaintext_len: usize,
    artefact: u8,
    case_pk: *const c_uchar,
    case_pk_len: usize,
    device_sk: *const c_uchar,
    device_sk_len: usize,
    out_env: *mut *mut c_uchar,
    out_env_len: *mut usize,
) -> c_int {
    if plaintext.is_null() || case_pk.is_null() || device_sk.is_null() {
        return ERR;
    }
    let pt = slice::from_raw_parts(plaintext, plaintext_len);
    let cpk = slice::from_raw_parts(case_pk, case_pk_len);
    let dsk = slice::from_raw_parts(device_sk, device_sk_len);
    let kind = match artefact {
        0 => ArtefactKind::AndroidBugreport,
        1 => ArtefactKind::IosSysdiagnose,
        2 => ArtefactKind::MessageReport,
        _ => return ERR,
    };
    let device = match DeviceKeyPair::from_private_bytes(dsk) {
        Ok(d) => d,
        Err(_) => return ERR,
    };
    match seal(pt, kind, cpk, &device) {
        Ok(env) => {
            write_buf(out_env, out_env_len, &env);
            OK
        }
        Err(_) => ERR,
    }
}

unsafe fn write_buf(out_ptr: *mut *mut c_uchar, out_len: *mut usize, data: &[u8]) {
    let mut v = data.to_vec();
    let len = v.len();
    let ptr = v.as_mut_ptr();
    std::mem::forget(v);
    *out_ptr = ptr;
    *out_len = len;
}

/// Used by tests / embedders that need a non-null sentinel.
#[no_mangle]
pub extern "C" fn cm_abi_version() -> c_int {
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr;

    #[test]
    fn abi_version() {
        assert_eq!(cm_abi_version(), 1);
    }

    #[test]
    fn seal_roundtrip_via_ffi_ptrs() {
        unsafe {
            let mut pk: *mut c_uchar = ptr::null_mut();
            let mut pk_len = 0usize;
            let mut sk: *mut c_uchar = ptr::null_mut();
            let mut sk_len = 0usize;
            assert_eq!(
                cm_generate_case_keypair(&mut pk, &mut pk_len, &mut sk, &mut sk_len),
                OK
            );

            let mut dpk: *mut c_uchar = ptr::null_mut();
            let mut dpk_len = 0usize;
            let mut dsk: *mut c_uchar = ptr::null_mut();
            let mut dsk_len = 0usize;
            assert_eq!(
                cm_generate_device_keypair(&mut dpk, &mut dpk_len, &mut dsk, &mut dsk_len),
                OK
            );

            let msg = b"ffi seal test";
            let mut env: *mut c_uchar = ptr::null_mut();
            let mut env_len = 0usize;
            assert_eq!(
                cm_seal(
                    msg.as_ptr(),
                    msg.len(),
                    0,
                    pk,
                    pk_len,
                    dsk,
                    dsk_len,
                    &mut env,
                    &mut env_len,
                ),
                OK
            );
            assert!(env_len > 8);
            assert_eq!(slice::from_raw_parts(env, 4), b"CM01");

            cm_free(pk, pk_len);
            cm_free(sk, sk_len);
            cm_free(dpk, dpk_len);
            cm_free(dsk, dsk_len);
            cm_free(env, env_len);
        }
    }
}
