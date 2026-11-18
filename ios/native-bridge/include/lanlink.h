/*
 * LANLink native core: C ABI over lanlink-mobile (Rust).
 *
 * Hand-written; kept in sync with ios/native-bridge/src/lib.rs by
 * scripts/check-ios-header.py.
 *
 * Conventions:
 *  - Every function returns int32_t: 0 = ok, -1 = error. The error text is
 *    written to *error_out (may be NULL). On success *error_out is set to NULL.
 *  - Outputs are untouched when a call fails.
 *  - Strings in are NUL-terminated UTF-8; NULL or invalid UTF-8 is an error.
 *  - Strings out are released with lanlink_string_free, byte buffers with
 *    lanlink_bytes_free (both NULL-safe).
 *  - Sessions are opaque int64_t handles; a stale handle is an error.
 *  - Blocking calls (connect, poll_event, next_video) must run on worker
 *    threads, never the main thread.
 */
#ifndef LANLINK_H
#define LANLINK_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ---- memory ---- */
void lanlink_string_free(char *s);
void lanlink_bytes_free(uint8_t *p, size_t len);

/* ---- identity ---- */
int32_t lanlink_generate_identity(const char *label, uint8_t **out, size_t *out_len, char **error_out);
int32_t lanlink_identity_fingerprint(const uint8_t *id, size_t len, char **out, char **error_out);
int32_t lanlink_parse_pairing_uri(const char *uri, char **out_json, char **error_out);

/* ---- pairing ---- */
int32_t lanlink_pair_start(const uint8_t *id, size_t len, const char *name, const char *model,
                           const char *host, int32_t port, const char *fp_hex, const char *code,
                           int64_t *out_job, char **error_out);
int32_t lanlink_pair_poll(int64_t job, char **out_json, char **error_out);
void lanlink_pair_cancel(int64_t job);

/* ---- sessions ---- */
int32_t lanlink_connect(const uint8_t *id, size_t len, const char *name, const char *model,
                        const char *host, int32_t port, const char *fp_hex,
                        int64_t *out_handle, char **error_out);
void lanlink_close(int64_t h);
int32_t lanlink_send_capabilities(int64_t h, int32_t max_w, int32_t max_h, int32_t max_fps,
                                  bool hardware, bool low_latency, int32_t display_w,
                                  int32_t display_h, int32_t refresh_millihz, char **error_out);
int32_t lanlink_start_session(int64_t h, int64_t display_id, int32_t quality, int32_t max_w,
                              int32_t max_h, int32_t fps, int32_t kbps, int32_t max_kbps,
                              bool view_only, char **error_out);
int32_t lanlink_set_stream_params(int64_t h, int32_t quality, int32_t max_w, int32_t max_h,
                                  int32_t fps, int32_t kbps, int32_t max_kbps, char **error_out);
int32_t lanlink_end_session(int64_t h, char **error_out);
int32_t lanlink_select_display(int64_t h, int64_t display_id, char **error_out);
int32_t lanlink_set_text_input(int64_t h, bool active, bool *out, char **error_out);
int32_t lanlink_bind_session(int64_t h, int64_t session_id, char **error_out);
int32_t lanlink_send_client_stats(int64_t h, int32_t received_fps_x10, int32_t rendered_fps_x10,
                                  int32_t decode_p50_us, int32_t decode_p95_us, int32_t dropped,
                                  int32_t queue, char **error_out);

/* ---- events ---- */
/* *out_json is NULL on timeout. */
int32_t lanlink_poll_event(int64_t h, int32_t timeout_ms, char **out_json, char **error_out);
/* *out is NULL when the shape is unknown; otherwise release with lanlink_bytes_free(*out, *out_len). */
int32_t lanlink_cursor_pixels(int64_t h, int64_t shape_id, uint8_t **out, size_t *out_len,
                              char **error_out);

/* ---- video ---- */
/*
 * Writes the next decoder input into buf (cap bytes). *out_code is the byte
 * count (>= 0) or a negative code: -1 timeout, -2 new stream configuration,
 * -3 closed, -4 buffer too small. meta receives only the entries the result
 * defines: 0 for -1/-3, 7 for -2, 1 for -4, 5 otherwise; see lanlink-mobile.
 */
int32_t lanlink_next_video(int64_t h, uint8_t *buf, size_t cap, int64_t meta[8], int32_t timeout_ms,
                           int32_t *out_code, char **error_out);
int32_t lanlink_decoder_reset(int64_t h, char **error_out);
int32_t lanlink_request_keyframe(int64_t h, char **error_out);
int64_t lanlink_now_us(void);

/* ---- input ---- */
int32_t lanlink_send_pointer(int64_t h, int32_t x, int32_t y, char **error_out);
int32_t lanlink_send_pointer_rel(int64_t h, int32_t dx, int32_t dy, char **error_out);
int32_t lanlink_send_button(int64_t h, int32_t button, bool down, int32_t x, int32_t y, bool *out,
                            char **error_out);
int32_t lanlink_send_scroll(int64_t h, int32_t dx, int32_t dy, bool *out, char **error_out);
int32_t lanlink_send_key(int64_t h, int32_t vk, bool down, bool *out, char **error_out);
int32_t lanlink_send_text(int64_t h, const char *text, bool *out, char **error_out);
int32_t lanlink_release_all(int64_t h, char **error_out);

#ifdef __cplusplus
}
#endif

#endif /* LANLINK_H */
