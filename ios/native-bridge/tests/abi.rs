//! Calls the `extern "C"` surface from Rust, the way Swift will.

use lanlink_ios::*;
use std::ffi::{c_char, CStr, CString};
use std::ptr;

unsafe fn take_string(p: *mut c_char) -> String {
    assert!(!p.is_null());
    let s = CStr::from_ptr(p).to_string_lossy().into_owned();
    lanlink_string_free(p);
    s
}

#[test]
fn null_and_bad_utf8_are_errors_not_crashes() {
    unsafe {
        let mut out: *mut u8 = ptr::null_mut();
        let mut len = 0usize;
        let mut err: *mut c_char = ptr::null_mut();
        assert_eq!(
            lanlink_generate_identity(ptr::null(), &mut out, &mut len, &mut err),
            -1
        );
        assert!(out.is_null());
        assert!(!take_string(err).is_empty());

        let bad = [0xffu8, 0xfe, 0];
        let mut err: *mut c_char = ptr::null_mut();
        assert_eq!(
            lanlink_generate_identity(bad.as_ptr().cast(), &mut out, &mut len, &mut err),
            -1
        );
        assert!(!take_string(err).is_empty());

        // `error_out` may be NULL.
        assert_eq!(
            lanlink_generate_identity(ptr::null(), &mut out, &mut len, ptr::null_mut()),
            -1
        );
        let mut json: *mut c_char = ptr::null_mut();
        assert_eq!(
            lanlink_parse_pairing_uri(ptr::null(), &mut json, ptr::null_mut()),
            -1
        );
    }
}

#[test]
fn errors_and_outputs_are_freeable() {
    unsafe {
        let label = CString::new("test phone").unwrap();
        let mut id: *mut u8 = ptr::null_mut();
        let mut id_len = 0usize;
        // Pre-set err to garbage-free non-null sentinel to prove it is reset.
        let mut err: *mut c_char = ptr::dangling_mut::<c_char>();
        assert_eq!(
            lanlink_generate_identity(label.as_ptr(), &mut id, &mut id_len, &mut err),
            0
        );
        assert!(err.is_null());
        assert!(!id.is_null() && id_len > 0);

        let mut fp: *mut c_char = ptr::null_mut();
        assert_eq!(
            lanlink_identity_fingerprint(id, id_len, &mut fp, &mut err),
            0
        );
        assert!(err.is_null());
        let fp = take_string(fp);
        assert!(!fp.is_empty());

        lanlink_bytes_free(id, id_len);
        lanlink_bytes_free(ptr::null_mut(), 0);
        lanlink_string_free(ptr::null_mut());
    }
}

#[test]
fn stale_handle_returns_error() {
    unsafe {
        let mut ok = true;
        let mut err: *mut c_char = ptr::null_mut();
        assert_eq!(lanlink_send_key(12345, 0x41, true, &mut ok, &mut err), -1);
        assert_eq!(take_string(err), "connection closed");

        let mut json: *mut c_char = ptr::null_mut();
        let mut err: *mut c_char = ptr::null_mut();
        assert_eq!(lanlink_poll_event(12345, 0, &mut json, &mut err), -1);
        assert_eq!(take_string(err), "connection closed");

        let mut buf = [0u8; 16];
        let mut meta = [7i64; 8];
        let mut code = 0i32;
        let mut err: *mut c_char = ptr::null_mut();
        assert_eq!(
            lanlink_next_video(
                12345,
                buf.as_mut_ptr(),
                buf.len(),
                meta.as_mut_ptr(),
                0,
                &mut code,
                &mut err
            ),
            -1
        );
        assert_eq!(take_string(err), "connection closed");
        lanlink_close(12345);
    }
}

#[test]
fn pairing_uri_errors_are_readable_and_now_us_ticks() {
    unsafe {
        let uri = CString::new("not a lanlink uri").unwrap();
        let mut json: *mut c_char = ptr::null_mut();
        let mut err: *mut c_char = ptr::null_mut();
        assert_eq!(
            lanlink_parse_pairing_uri(uri.as_ptr(), &mut json, &mut err),
            -1
        );
        assert!(!take_string(err).is_empty());
        let a = lanlink_now_us();
        assert!(a >= 0 && lanlink_now_us() >= a);
    }
}
