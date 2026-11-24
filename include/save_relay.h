/*
 * save-relay C interface for Android and iOS apps (cargo feature `ffi`, src/ffi.rs).
 * Written by hand; tests/ffi_header.rs checks every exported function appears here.
 *
 * Every function that returns `char *` returns one line of JSON, the same envelope as
 * `save-relay <command> --json` (docs/code-map.md, "API shape"), and the caller frees it with
 * save_relay_free. A NULL or non-UTF-8 argument gives an error envelope.
 *
 * Call save_relay_init once first. The app supplies its folders, its credential store (the only
 * place a password or token is written) and its network.
 */
#ifndef SAVE_RELAY_H
#define SAVE_RELAY_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Where the app's `http` callback puts its answer. Opaque. */
typedef struct SaveRelayHttpSink SaveRelayHttpSink;

/* One HTTP request for the `http` callback; pointers are valid only during the call. */
typedef struct SaveRelayHttpRequest {
  const char *method;
  const char *url;
  /* "Name: value" lines, each ended by '\n'. */
  const char *headers;
  const uint8_t *body;
  size_t body_len;
  /* Stop reading the answer after this many bytes and fail; 0 means no bound. */
  size_t max_response_bytes;
} SaveRelayHttpRequest;

typedef struct SaveRelayHost {
  /* Passed back to every callback. */
  void *user_data;
  /* The app's folders (copied during save_relay_init). `documents` may be NULL. */
  const char *home;
  const char *data;
  const char *config;
  const char *documents;
  /* Write the secret for `account` into `buf` (at most `cap` bytes) and its length into `*len`.
   * Return 0 when found, 1 when there is none, anything else for a failure. */
  int (*cred_get)(void *user_data, const char *account, uint8_t *buf, size_t cap, size_t *len);
  /* Store (or replace) `secret` under `account`. Return 0 on success. */
  int (*cred_set)(void *user_data, const char *account, const char *secret);
  /* Remove `account`; a missing one is success. Return 0 on success. */
  int (*cred_delete)(void *user_data, const char *account);
  /* Send `request`, call save_relay_http_respond(sink, ...) with the answer (any status) and
   * return 0. Return nonzero for a connection, DNS, TLS or timeout failure. */
  int (*http)(void *user_data, const SaveRelayHttpRequest *request, SaveRelayHttpSink *sink);
} SaveRelayHost;

/* For the `http` callback: hand over the answer. `headers` is "Name: value" lines or NULL;
 * `body` may be NULL with body_len 0. Returns 0, or 1 on invalid input or panic.
 * Non-UTF-8 headers and NULL bodies with nonzero lengths are refused. */
int save_relay_http_respond(SaveRelayHttpSink *sink, uint16_t status, const char *headers,
                            const uint8_t *body, size_t body_len);

/* Tell the library what the app supplies. A second call replaces the first. */
char *save_relay_init(const SaveRelayHost *host);

/* `resolve` is NULL, "keep-server" or "keep-local" (the answer to a `conflict`). */
char *save_relay_pull(const char *manifest_json, const char *saves_dir, const char *resolve);
char *save_relay_push(const char *manifest_json, const char *saves_dir, const char *resolve);
char *save_relay_status(const char *manifest_json, const char *saves_dir);

/* Nextcloud setup, once per machine. The password goes to `cred_set` only. `device` and `root`
 * may be NULL. */
char *save_relay_setup(const char *server, const char *user, const char *password,
                       const char *device, const char *root);

/* Google Drive or Dropbox sign-in through the app's own browser session (the same PKCE parts as
 * the desktop sign-in). `service` is "gdrive" or "dropbox"; `redirect_uri` is the app's redirect.
 * The answer's data.url is the page the app opens (its code is NULL); when the browser session
 * returns, pass that redirect URL to save_relay_sign_in_finish. On Android 15+, return the app
 * to the foreground before calling save_relay_sign_in_finish (backgrounded apps without a
 * foreground service have outbound DNS/HTTP blocked); on `server_unreachable` or `server_error`,
 * the pending sign-in is kept so the app can retry with the same redirect URL. The refresh token
 * goes to `cred_set` only. `device` and `root` may be NULL. */
char *save_relay_sign_in_start(const char *service, const char *redirect_uri, const char *device,
                               const char *root);
char *save_relay_sign_in_finish(const char *redirect_url);

/* Game files (docs/assets.md, #305): resumable download of an app's files from the signed-in
 * Dropbox, `game-files/<app_id>/manifest.json` and `game-files/<app_id>/files/...` in the App
 * folder. Needs a Dropbox sign-in (save_relay_sign_in_start/_finish). `dest_dir` is where the
 * files go and `state_dir` is where the library keeps its checkpoints and `.part` files: both
 * app-private, on the same file system, neither inside the other and neither the saves folder.
 * Each answer is the usual JSON envelope (commands "game-files-plan", "game-files-download",
 * "game-files-progress", "game-files-cancel", "game-files-cleanup"); an error envelope also has
 * error.class: "retryable", "not_yet_uploaded", "reauth_needed" or "fatal". Every request asks
 * for a bounded range through the `http` callback, which must send the Range header, return the
 * 206 answer with its Content-Range and Dropbox-API-Result headers untouched, send no
 * Accept-Encoding but identity, and honour max_response_bytes. */

/* What a download would do, from file sizes only; changes nothing. data: files and bytes
 * present, partial and missing, and the files in the way. Blocks for one request. */
char *save_relay_game_files_plan(const char *app_id, const char *dest_dir, const char *state_dir);

/* Download or resume everything missing. Blocks until it finishes, fails or is cancelled, so
 * call it from a background thread. A killed process, a dead battery or a dropped network loses
 * at most one chunk; call it again to continue. data: the final progress, state "complete" or
 * "cancelled". One download runs at a time (error code "busy"). */
char *save_relay_game_files_download(const char *app_id, const char *dest_dir,
                                     const char *state_dir);

/* data: the latest progress of the running or last download (state, files_done, files_total,
 * bytes_done, bytes_total, current), or null before any. Cheap; poll it twice a second. */
char *save_relay_game_files_progress(void);

/* Stop the running download after its current chunk is durable; the download call returns
 * "cancelled" and a later download continues. data.running says whether one was running. */
char *save_relay_game_files_cancel(void);

/* Remove the checkpoints and `.part` files in `state_dir` (never the game folder), so the next
 * download starts unfinished files again. Refused (error code "busy") while a download runs. */
char *save_relay_game_files_cleanup(const char *state_dir);

/* Free a string returned above. NULL is ignored. */
void save_relay_free(char *pointer);

#ifdef __cplusplus
}
#endif

#endif /* SAVE_RELAY_H */
