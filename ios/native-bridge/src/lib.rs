//! C ABI bridge between the iOS app and `lanlink-mobile`.
//!
//! This file is glue only: every entry point converts its arguments, calls
//! the platform-neutral core, and writes the result through out-pointers.
//! The matching header is `include/lanlink.h`; `scripts/check-ios-header.py`
//! keeps the two in sync.
//!
//! Boundary rules:
//!
//! * Every function returns `int32_t` status: `0` ok, `-1` error. The error
//!   text goes to `error_out` (may be NULL; set to NULL on success).
//! * Every entry point catches panics and reports them as
//!   "internal error in LANLink native code".
//! * Strings in are NUL-terminated UTF-8; NULL or invalid UTF-8 is an error.
//! * Strings and byte buffers out are allocated here and must be released
//!   with `lanlink_string_free` / `lanlink_bytes_free` (both NULL-safe).
//! * Sessions are opaque `i64` handles; a stale handle is an error.
//! * Video is copied into a caller-provided buffer.
//! * Blocking calls take a timeout and must run on worker threads.
//!
//! Outputs are left untouched when a call fails.

#![allow(clippy::missing_safety_doc)]

use lanlink_mobile as mobile;
use std::ffi::{c_char, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};

const INTERNAL: &str = "internal error in LANLink native code";

// ---------------------------------------------------------------- helpers

unsafe fn set_error(error_out: *mut *mut c_char, msg: &str) {
    if error_out.is_null() {
        return;
    }
    // Interior NULs cannot cross as a C string; replace them.
    let c = CString::new(msg.replace('\0', " ")).unwrap_or_default();
    *error_out = c.into_raw();
}

/// Run `f` behind a panic guard and map it to a status code.
unsafe fn call(error_out: *mut *mut c_char, f: impl FnOnce() -> Result<(), String>) -> i32 {
    if !error_out.is_null() {
        *error_out = std::ptr::null_mut();
    }
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(())) => 0,
        Ok(Err(msg)) => {
            set_error(error_out, &msg);
            -1
        }
        Err(_) => {
            set_error(error_out, INTERNAL);
            -1
        }
    }
}

unsafe fn cstr<'a>(p: *const c_char, what: &str) -> Result<&'a str, String> {
    if p.is_null() {
        return Err(format!("{what} is missing"));
    }
    CStr::from_ptr(p)
        .to_str()
        .map_err(|_| format!("{what} is not valid UTF-8"))
}

unsafe fn bytes_in<'a>(p: *const u8, len: usize, what: &str) -> Result<&'a [u8], String> {
    if p.is_null() {
        return Err(format!("{what} is missing"));
    }
    Ok(std::slice::from_raw_parts(p, len))
}

fn out_ptr<'a, T>(p: *mut T, what: &str) -> Result<&'a mut T, String> {
    // SAFETY: the caller passes a valid, writable pointer or NULL.
    unsafe { p.as_mut() }.ok_or_else(|| format!("{what} is missing"))
}

fn into_c_string(s: String) -> Result<*mut c_char, String> {
    CString::new(s)
        .map(CString::into_raw)
        .map_err(|_| "text contained a NUL byte".to_string())
}

fn into_c_bytes(v: Vec<u8>, len_out: &mut usize) -> *mut u8 {
    *len_out = v.len();
    Box::into_raw(v.into_boxed_slice()) as *mut u8
}

// ---------------------------------------------------------------- memory

/// Free a string returned by this library. NULL is fine.
#[no_mangle]
pub unsafe extern "C" fn lanlink_string_free(s: *mut c_char) {
    if !s.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| drop(CString::from_raw(s))));
    }
}

/// Free a byte buffer returned by this library, with its length. NULL is fine.
#[no_mangle]
pub unsafe extern "C" fn lanlink_bytes_free(p: *mut u8, len: usize) {
    if !p.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(p, len)))
        }));
    }
}

// ---------------------------------------------------------------- identity

#[no_mangle]
pub unsafe extern "C" fn lanlink_generate_identity(
    label: *const c_char,
    out: *mut *mut u8,
    out_len: *mut usize,
    error_out: *mut *mut c_char,
) -> i32 {
    call(error_out, || {
        let label = cstr(label, "label")?;
        let (out, out_len) = (out_ptr(out, "out")?, out_ptr(out_len, "out_len")?);
        *out = into_c_bytes(mobile::generate_identity(label)?, out_len);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn lanlink_identity_fingerprint(
    id: *const u8,
    len: usize,
    out: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> i32 {
    call(error_out, || {
        let id = bytes_in(id, len, "identity")?;
        let out = out_ptr(out, "out")?;
        *out = into_c_string(mobile::identity_fingerprint(id)?)?;
        Ok(())
    })
}

/// Parse a scanned QR link into JSON, or fail with a readable reason.
#[no_mangle]
pub unsafe extern "C" fn lanlink_parse_pairing_uri(
    uri: *const c_char,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> i32 {
    call(error_out, || {
        let uri = cstr(uri, "uri")?;
        let out = out_ptr(out_json, "out_json")?;
        *out = into_c_string(mobile::parse_pairing_uri(uri)?)?;
        Ok(())
    })
}

// ---------------------------------------------------------------- pairing

#[no_mangle]
pub unsafe extern "C" fn lanlink_pair_start(
    id: *const u8,
    len: usize,
    name: *const c_char,
    model: *const c_char,
    host: *const c_char,
    port: i32,
    fp_hex: *const c_char,
    code: *const c_char,
    out_job: *mut i64,
    error_out: *mut *mut c_char,
) -> i32 {
    call(error_out, || {
        let id = bytes_in(id, len, "identity")?;
        let name = cstr(name, "name")?;
        let model = cstr(model, "model")?;
        let host = cstr(host, "host")?;
        let fp = cstr(fp_hex, "fingerprint")?;
        let code = cstr(code, "code")?;
        let out = out_ptr(out_job, "out_job")?;
        *out = mobile::pair_start(id, name, model, host, port, fp, code)?;
        Ok(())
    })
}

/// JSON: `{state:"connecting"|"confirm"|"done"|"failed", code?, hostName?,
/// fingerprint?, error?}`. A finished job is removed after it is reported.
#[no_mangle]
pub unsafe extern "C" fn lanlink_pair_poll(
    job: i64,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> i32 {
    call(error_out, || {
        let out = out_ptr(out_json, "out_json")?;
        *out = into_c_string(mobile::pair_poll(job)?)?;
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn lanlink_pair_cancel(job: i64) {
    let _ = catch_unwind(|| mobile::pair_cancel(job));
}

// ---------------------------------------------------------------- sessions

/// Blocking connect to a paired host.
#[no_mangle]
pub unsafe extern "C" fn lanlink_connect(
    id: *const u8,
    len: usize,
    name: *const c_char,
    model: *const c_char,
    host: *const c_char,
    port: i32,
    fp_hex: *const c_char,
    out_handle: *mut i64,
    error_out: *mut *mut c_char,
) -> i32 {
    call(error_out, || {
        let id = bytes_in(id, len, "identity")?;
        let name = cstr(name, "name")?;
        let model = cstr(model, "model")?;
        let host = cstr(host, "host")?;
        let fp = cstr(fp_hex, "fingerprint")?;
        let out = out_ptr(out_handle, "out_handle")?;
        *out = mobile::connect(id, name, model, host, port, fp)?;
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn lanlink_close(h: i64) {
    let _ = catch_unwind(|| mobile::close(h));
}

#[no_mangle]
pub unsafe extern "C" fn lanlink_send_capabilities(
    h: i64,
    max_w: i32,
    max_h: i32,
    max_fps: i32,
    hardware: bool,
    low_latency: bool,
    display_w: i32,
    display_h: i32,
    refresh_millihz: i32,
    error_out: *mut *mut c_char,
) -> i32 {
    call(error_out, || {
        mobile::send_capabilities(
            h,
            max_w,
            max_h,
            max_fps,
            hardware,
            low_latency,
            display_w,
            display_h,
            refresh_millihz,
        )
    })
}

#[no_mangle]
pub unsafe extern "C" fn lanlink_start_session(
    h: i64,
    display_id: i64,
    quality: i32,
    max_w: i32,
    max_h: i32,
    fps: i32,
    kbps: i32,
    max_kbps: i32,
    view_only: bool,
    error_out: *mut *mut c_char,
) -> i32 {
    call(error_out, || {
        mobile::start_session(
            h, display_id, quality, max_w, max_h, fps, kbps, max_kbps, view_only,
        )
    })
}

#[no_mangle]
pub unsafe extern "C" fn lanlink_set_stream_params(
    h: i64,
    quality: i32,
    max_w: i32,
    max_h: i32,
    fps: i32,
    kbps: i32,
    max_kbps: i32,
    error_out: *mut *mut c_char,
) -> i32 {
    call(error_out, || {
        mobile::set_stream_params(h, quality, max_w, max_h, fps, kbps, max_kbps)
    })
}

#[no_mangle]
pub unsafe extern "C" fn lanlink_end_session(h: i64, error_out: *mut *mut c_char) -> i32 {
    call(error_out, || mobile::end_session(h))
}

#[no_mangle]
pub unsafe extern "C" fn lanlink_select_display(
    h: i64,
    display_id: i64,
    error_out: *mut *mut c_char,
) -> i32 {
    call(error_out, || mobile::select_display(h, display_id))
}

/// The phone keyboard opened or closed. `*out` is false if the host is too
/// old to report the caret (protocol < 2) or the queue was full.
#[no_mangle]
pub unsafe extern "C" fn lanlink_set_text_input(
    h: i64,
    active: bool,
    out: *mut bool,
    error_out: *mut *mut c_char,
) -> i32 {
    call(error_out, || {
        let out = out_ptr(out, "out")?;
        *out = mobile::set_text_input(h, active)?;
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn lanlink_bind_session(
    h: i64,
    session_id: i64,
    error_out: *mut *mut c_char,
) -> i32 {
    call(error_out, || mobile::bind_session(h, session_id))
}

#[no_mangle]
pub unsafe extern "C" fn lanlink_send_client_stats(
    h: i64,
    received_fps_x10: i32,
    rendered_fps_x10: i32,
    decode_p50_us: i32,
    decode_p95_us: i32,
    dropped: i32,
    queue: i32,
    error_out: *mut *mut c_char,
) -> i32 {
    call(error_out, || {
        mobile::send_client_stats(
            h,
            received_fps_x10,
            rendered_fps_x10,
            decode_p50_us,
            decode_p95_us,
            dropped,
            queue,
        )
    })
}

// ---------------------------------------------------------------- events

/// Wait up to `timeout_ms` for the next control event. `*out_json` is JSON,
/// or NULL on timeout.
#[no_mangle]
pub unsafe extern "C" fn lanlink_poll_event(
    h: i64,
    timeout_ms: i32,
    out_json: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> i32 {
    call(error_out, || {
        let out = out_ptr(out_json, "out_json")?;
        *out = match mobile::poll_event(h, timeout_ms)? {
            Some(json) => into_c_string(json)?,
            None => std::ptr::null_mut(),
        };
        Ok(())
    })
}

/// RGBA pixels of a cursor shape announced by a `cursorShape` event.
/// `*out` is NULL when the shape is unknown.
#[no_mangle]
pub unsafe extern "C" fn lanlink_cursor_pixels(
    h: i64,
    shape_id: i64,
    out: *mut *mut u8,
    out_len: *mut usize,
    error_out: *mut *mut c_char,
) -> i32 {
    call(error_out, || {
        let (out, out_len) = (out_ptr(out, "out")?, out_ptr(out_len, "out_len")?);
        match mobile::cursor_pixels(h, shape_id)? {
            Some(rgba) => *out = into_c_bytes(rgba, out_len),
            None => {
                *out = std::ptr::null_mut();
                *out_len = 0;
            }
        }
        Ok(())
    })
}

// ---------------------------------------------------------------- video

/// Write the next decoder input into `buf` (`cap` bytes). `*out_code` is the
/// `next_video` result: the byte count (>= 0) or a negative code, and `meta`
/// receives exactly the entries that result defines (`meta_len`). See
/// `lanlink_mobile::next_video` for the codes and the `meta` layout.
#[no_mangle]
pub unsafe extern "C" fn lanlink_next_video(
    h: i64,
    buf: *mut u8,
    cap: usize,
    meta: *mut i64,
    timeout_ms: i32,
    out_code: *mut i32,
    error_out: *mut *mut c_char,
) -> i32 {
    call(error_out, || {
        let out_code = out_ptr(out_code, "out_code")?;
        if meta.is_null() {
            return Err("meta is missing".into());
        }
        let buf: &mut [u8] = if cap == 0 {
            &mut []
        } else if buf.is_null() {
            return Err("buf is missing".into());
        } else {
            std::slice::from_raw_parts_mut(buf, cap)
        };
        let mut m = [0i64; 8];
        let ret = mobile::next_video(h, buf, &mut m, timeout_ms)?;
        let n = mobile::meta_len(ret);
        std::ptr::copy_nonoverlapping(m.as_ptr(), meta, n);
        *out_code = ret;
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn lanlink_decoder_reset(h: i64, error_out: *mut *mut c_char) -> i32 {
    call(error_out, || mobile::decoder_reset(h))
}

#[no_mangle]
pub unsafe extern "C" fn lanlink_request_keyframe(h: i64, error_out: *mut *mut c_char) -> i32 {
    call(error_out, || mobile::request_keyframe(h))
}

/// Monotonic microseconds on the same clock as `recvUs` in video metadata.
#[no_mangle]
pub extern "C" fn lanlink_now_us() -> i64 {
    catch_unwind(mobile::now_us).unwrap_or(0)
}

// ---------------------------------------------------------------- input

#[no_mangle]
pub unsafe extern "C" fn lanlink_send_pointer(
    h: i64,
    x: i32,
    y: i32,
    error_out: *mut *mut c_char,
) -> i32 {
    call(error_out, || mobile::send_pointer(h, x, y))
}

#[no_mangle]
pub unsafe extern "C" fn lanlink_send_pointer_rel(
    h: i64,
    dx: i32,
    dy: i32,
    error_out: *mut *mut c_char,
) -> i32 {
    call(error_out, || mobile::send_pointer_rel(h, dx, dy))
}

/// `button`: 0 left, 1 right, 2 middle. `x`/`y` < 0 means "no position".
/// `*out` is false when the input queue was full.
#[no_mangle]
pub unsafe extern "C" fn lanlink_send_button(
    h: i64,
    button: i32,
    down: bool,
    x: i32,
    y: i32,
    out: *mut bool,
    error_out: *mut *mut c_char,
) -> i32 {
    call(error_out, || {
        let out = out_ptr(out, "out")?;
        *out = mobile::send_button(h, button, down, x, y)?;
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn lanlink_send_scroll(
    h: i64,
    dx: i32,
    dy: i32,
    out: *mut bool,
    error_out: *mut *mut c_char,
) -> i32 {
    call(error_out, || {
        let out = out_ptr(out, "out")?;
        *out = mobile::send_scroll(h, dx, dy)?;
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn lanlink_send_key(
    h: i64,
    vk: i32,
    down: bool,
    out: *mut bool,
    error_out: *mut *mut c_char,
) -> i32 {
    call(error_out, || {
        let out = out_ptr(out, "out")?;
        *out = mobile::send_key(h, vk, down)?;
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn lanlink_send_text(
    h: i64,
    text: *const c_char,
    out: *mut bool,
    error_out: *mut *mut c_char,
) -> i32 {
    call(error_out, || {
        let text = cstr(text, "text")?;
        let out = out_ptr(out, "out")?;
        *out = mobile::send_text(h, text)?;
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn lanlink_release_all(h: i64, error_out: *mut *mut c_char) -> i32 {
    call(error_out, || mobile::release_all(h))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test-only entry that panics inside the guard.
    #[allow(non_snake_case)]
    unsafe extern "C" fn lanlink__test_panic(error_out: *mut *mut c_char) -> i32 {
        call(error_out, || -> Result<(), String> { panic!("boom") })
    }

    #[test]
    fn panics_become_errors() {
        unsafe {
            let mut err: *mut c_char = std::ptr::null_mut();
            assert_eq!(lanlink__test_panic(&mut err), -1);
            assert_eq!(
                CStr::from_ptr(err).to_str().unwrap(),
                "internal error in LANLink native code"
            );
            lanlink_string_free(err);
            // NULL error_out must also be survivable.
            assert_eq!(lanlink__test_panic(std::ptr::null_mut()), -1);
        }
    }
}
