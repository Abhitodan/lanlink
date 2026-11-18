//! JNI bridge between the Android app and `lanlink-mobile`.
//!
//! This file is glue only: every entry point converts its arguments, calls
//! the platform-neutral core, and maps the result back to Java.
//!
//! Boundary rules:
//!
//! * Every entry point catches panics; Rust errors become Java exceptions
//!   (`java.io.IOException` with a user-presentable message).
//! * Sessions live in a registry keyed by an opaque `long` handle. Kotlin
//!   never holds a raw pointer, so a stale handle is an error, not a crash.
//! * Video never crosses as a Java `byte[]`: `nextVideo` writes the access
//!   unit straight into the `MediaCodec` input buffer (a direct
//!   `ByteBuffer`), which is the one copy decoding needs anyway.
//! * Control-plane events (low rate) cross as small JSON strings.
//! * Blocking calls take a timeout and are made from Kotlin worker threads,
//!   never from the main thread.

use jni::objects::{JByteArray, JByteBuffer, JClass, JLongArray, JString};
use jni::sys::{jboolean, jbyteArray, jint, jlong, jstring, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;
use lanlink_mobile as mobile;
use std::panic::AssertUnwindSafe;

// ---------------------------------------------------------------- errors

fn throw(env: &mut JNIEnv, msg: &str) {
    let _ = env.throw_new("java/io/IOException", msg);
}

/// Run `f`, converting panics and errors into a Java exception and
/// returning `default`.
fn guard<'l, T>(
    env: &mut JNIEnv<'l>,
    default: T,
    f: impl FnOnce(&mut JNIEnv<'l>) -> Result<T, String>,
) -> T {
    match std::panic::catch_unwind(AssertUnwindSafe(|| f(env))) {
        Ok(Ok(v)) => v,
        Ok(Err(msg)) => {
            throw(env, &msg);
            default
        }
        Err(_) => {
            throw(env, "internal error in LANLink native code");
            default
        }
    }
}

fn jstr(env: &mut JNIEnv, s: &JString) -> Result<String, String> {
    if s.is_null() {
        return Ok(String::new());
    }
    env.get_string(s).map(Into::into).map_err(|e| e.to_string())
}

fn bytes_in(env: &mut JNIEnv, a: &JByteArray) -> Result<Vec<u8>, String> {
    env.convert_byte_array(a).map_err(|e| e.to_string())
}

fn new_jstring(env: &mut JNIEnv, s: &str) -> jstring {
    env.new_string(s)
        .map(|s| s.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

fn jbool(b: bool) -> jboolean {
    if b {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

// ---------------------------------------------------------------- identity

#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_generateIdentity<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    label: JString<'l>,
) -> jbyteArray {
    guard(&mut env, std::ptr::null_mut(), |env| {
        let label = jstr(env, &label)?;
        let id = mobile::generate_identity(&label)?;
        env.byte_array_from_slice(&id)
            .map(|a| a.into_raw())
            .map_err(|e| e.to_string())
    })
}

#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_identityFingerprint<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    identity: JByteArray<'l>,
) -> jstring {
    guard(&mut env, std::ptr::null_mut(), |env| {
        let fp = mobile::identity_fingerprint(&bytes_in(env, &identity)?)?;
        Ok(new_jstring(env, &fp))
    })
}

/// Parse a scanned QR link. Returns JSON or throws with a readable reason.
#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_parsePairingUri<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    uri: JString<'l>,
) -> jstring {
    guard(&mut env, std::ptr::null_mut(), |env| {
        let s = jstr(env, &uri)?;
        let json = mobile::parse_pairing_uri(&s)?;
        Ok(new_jstring(env, &json))
    })
}

// ---------------------------------------------------------------- pairing

#[allow(clippy::too_many_arguments)]
#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_pairStart<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    identity: JByteArray<'l>,
    device_name: JString<'l>,
    device_model: JString<'l>,
    host: JString<'l>,
    port: jint,
    fingerprint_hex: JString<'l>,
    code: JString<'l>,
) -> jlong {
    guard(&mut env, 0, |env| {
        let id = bytes_in(env, &identity)?;
        let name = jstr(env, &device_name)?;
        let model = jstr(env, &device_model)?;
        let host = jstr(env, &host)?;
        let fp = jstr(env, &fingerprint_hex)?;
        let code = jstr(env, &code)?;
        mobile::pair_start(&id, &name, &model, &host, port, &fp, &code)
    })
}

/// JSON: `{state:"connecting"|"confirm"|"done"|"failed", code?, hostName?,
/// fingerprint?, error?}`. A finished job is removed after it is reported.
#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_pairPoll<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
) -> jstring {
    guard(&mut env, std::ptr::null_mut(), |env| {
        let json = mobile::pair_poll(handle)?;
        Ok(new_jstring(env, &json))
    })
}

#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_pairCancel<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
) {
    guard(&mut env, (), |_| {
        mobile::pair_cancel(handle);
        Ok(())
    })
}

// ---------------------------------------------------------------- sessions

/// Blocking connect to a paired host. Returns a handle.
#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_connect<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    identity: JByteArray<'l>,
    device_name: JString<'l>,
    device_model: JString<'l>,
    host: JString<'l>,
    port: jint,
    fingerprint_hex: JString<'l>,
) -> jlong {
    guard(&mut env, 0, |env| {
        let id = bytes_in(env, &identity)?;
        let name = jstr(env, &device_name)?;
        let model = jstr(env, &device_model)?;
        let host = jstr(env, &host)?;
        let fp = jstr(env, &fingerprint_hex)?;
        mobile::connect(&id, &name, &model, &host, port, &fp)
    })
}

#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_close<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
) {
    guard(&mut env, (), |_| {
        mobile::close(handle);
        Ok(())
    })
}

#[allow(clippy::too_many_arguments)]
#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_sendCapabilities<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
    max_w: jint,
    max_h: jint,
    max_fps: jint,
    hardware: jboolean,
    low_latency: jboolean,
    display_w: jint,
    display_h: jint,
    refresh_millihz: jint,
) {
    guard(&mut env, (), |_| {
        mobile::send_capabilities(
            handle,
            max_w,
            max_h,
            max_fps,
            hardware != JNI_FALSE,
            low_latency != JNI_FALSE,
            display_w,
            display_h,
            refresh_millihz,
        )
    })
}

#[allow(clippy::too_many_arguments)]
#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_startSession<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
    display_id: jlong,
    quality: jint,
    max_w: jint,
    max_h: jint,
    fps: jint,
    kbps: jint,
    max_kbps: jint,
    view_only: jboolean,
) {
    guard(&mut env, (), |_| {
        mobile::start_session(
            handle,
            display_id,
            quality,
            max_w,
            max_h,
            fps,
            kbps,
            max_kbps,
            view_only != JNI_FALSE,
        )
    })
}

#[allow(clippy::too_many_arguments)]
#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_setStreamParams<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
    quality: jint,
    max_w: jint,
    max_h: jint,
    fps: jint,
    kbps: jint,
    max_kbps: jint,
) {
    guard(&mut env, (), |_| {
        mobile::set_stream_params(handle, quality, max_w, max_h, fps, kbps, max_kbps)
    })
}

#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_endSession<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
) {
    guard(&mut env, (), |_| mobile::end_session(handle))
}

#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_selectDisplay<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
    display_id: jlong,
) {
    guard(&mut env, (), |_| mobile::select_display(handle, display_id))
}

/// The phone's keyboard opened or closed. Returns false if the host is too
/// old to report the caret (protocol < 2) or the queue was full.
#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_setTextInput<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
    active: jboolean,
) -> jboolean {
    guard(&mut env, 0, |_| {
        Ok(jbool(mobile::set_text_input(handle, active != 0)?))
    })
}

#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_bindSession<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
    session_id: jlong,
) {
    guard(&mut env, (), |_| mobile::bind_session(handle, session_id))
}

#[allow(clippy::too_many_arguments)]
#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_sendClientStats<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
    received_fps_x10: jint,
    rendered_fps_x10: jint,
    decode_p50_us: jint,
    decode_p95_us: jint,
    dropped: jint,
    queue: jint,
) {
    guard(&mut env, (), |_| {
        mobile::send_client_stats(
            handle,
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

/// Wait up to `timeout_ms` for the next control event. Returns JSON or
/// null on timeout.
#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_pollEvent<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
    timeout_ms: jint,
) -> jstring {
    guard(
        &mut env,
        std::ptr::null_mut(),
        |env| match mobile::poll_event(handle, timeout_ms)? {
            Some(json) => Ok(new_jstring(env, &json)),
            None => Ok(std::ptr::null_mut()),
        },
    )
}

/// RGBA pixels of a cursor shape announced by a `cursorShape` event.
#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_cursorPixels<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
    shape_id: jlong,
) -> jbyteArray {
    guard(
        &mut env,
        std::ptr::null_mut(),
        |env| match mobile::cursor_pixels(handle, shape_id)? {
            Some(rgba) => env
                .byte_array_from_slice(&rgba)
                .map(|a| a.into_raw())
                .map_err(|e| e.to_string()),
            None => Ok(std::ptr::null_mut()),
        },
    )
}

// ---------------------------------------------------------------- video

/// Write the next decoder input into `buffer` (a direct ByteBuffer, e.g. a
/// MediaCodec input buffer). See `lanlink_mobile::next_video` for the return
/// codes and the `meta` layout.
#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_nextVideo<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
    buffer: JByteBuffer<'l>,
    meta: JLongArray<'l>,
    timeout_ms: jint,
) -> jint {
    guard(&mut env, mobile::VIDEO_CLOSED, |env| {
        let cap = env
            .get_direct_buffer_capacity(&buffer)
            .map_err(|e| e.to_string())?;
        let ptr = env
            .get_direct_buffer_address(&buffer)
            .map_err(|e| e.to_string())?;
        // SAFETY: `ptr` is the start of a direct buffer of `cap` bytes that
        // the Kotlin caller owns for the duration of this call.
        let buf = unsafe { std::slice::from_raw_parts_mut(ptr, cap) };
        let mut m = [0i64; 8];
        let ret = mobile::next_video(handle, buf, &mut m, timeout_ms)?;
        // Copy back exactly the entries each result defines.
        let n = mobile::meta_len(ret);
        if n > 0 {
            env.set_long_array_region(&meta, 0, &m[..n])
                .map_err(|e| e.to_string())?;
        }
        Ok(ret)
    })
}

/// The app created a new decoder (first start, surface recreated, error
/// recovery): resend parameter sets and start from a fresh keyframe.
#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_decoderReset<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
) {
    guard(&mut env, (), |_| mobile::decoder_reset(handle))
}

#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_requestKeyframe<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
) {
    guard(&mut env, (), |_| mobile::request_keyframe(handle))
}

/// Monotonic microseconds on the same clock as `recvUs` in video metadata.
#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_nowUs<'l>(
    _env: JNIEnv<'l>,
    _c: JClass<'l>,
) -> jlong {
    mobile::now_us()
}

// ---------------------------------------------------------------- input

#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_sendPointer<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
    x: jint,
    y: jint,
) {
    guard(&mut env, (), |_| mobile::send_pointer(handle, x, y))
}

#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_sendPointerRel<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
    dx: jint,
    dy: jint,
) {
    guard(&mut env, (), |_| mobile::send_pointer_rel(handle, dx, dy))
}

/// `button`: 0 left, 1 right, 2 middle. `x`/`y` < 0 means "no position".
#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_sendButton<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
    button: jint,
    down: jboolean,
    x: jint,
    y: jint,
) -> jboolean {
    guard(&mut env, JNI_FALSE, |_| {
        Ok(jbool(mobile::send_button(
            handle,
            button,
            down != JNI_FALSE,
            x,
            y,
        )?))
    })
}

#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_sendScroll<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
    dx: jint,
    dy: jint,
) -> jboolean {
    guard(&mut env, JNI_FALSE, |_| {
        Ok(jbool(mobile::send_scroll(handle, dx, dy)?))
    })
}

#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_sendKey<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
    vk: jint,
    down: jboolean,
) -> jboolean {
    guard(&mut env, JNI_FALSE, |_| {
        Ok(jbool(mobile::send_key(handle, vk, down != JNI_FALSE)?))
    })
}

#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_sendText<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
    text: JString<'l>,
) -> jboolean {
    guard(&mut env, JNI_FALSE, |env| {
        let t = jstr(env, &text)?;
        Ok(jbool(mobile::send_text(handle, &t)?))
    })
}

#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_releaseAll<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
) {
    guard(&mut env, (), |_| mobile::release_all(handle))
}
