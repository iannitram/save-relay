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
  /* Read at most this many bytes plus one, then stop; 0 means no bound. Hand a longer answer
   * to save_relay_http_respond anyway (its body_len past this bound): it is refused and counted
   * as too large (a range the server ignored, or a manifest over its limit), not as a failed
   * connection to retry. */
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
 * Non-UTF-8 headers and NULL bodies with nonzero lengths are refused. A body_len past the
 * request's max_response_bytes is refused (1) and recorded as an answer that is too large,
 * whatever the callback returns afterwards. */
int save_relay_http_respond(SaveRelayHttpSink *sink, uint16_t status, const char *headers,
                            const uint8_t *body, size_t body_len);

/* Tell the library what the app supplies. A second call replaces the first, except while a
 * game-file download runs: then it answers error code "busy" and keeps the first. */
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

/* ---------------------------------------------------------------------------------------------
 * Game files (docs/assets.md, #305, #309, #310): resumable download of an app's files.
 *
 * VERSION. save_relay_game_files_version() returns the version of this part of the interface
 * (1). Within one version no signature, envelope command or error code listed here changes; a
 * host built against this header may compare the number at start-up and refuse a library that
 * answers something else.
 *
 * SERVICE. The files come from whichever service this device is signed in to or set up for:
 * `game-files/<app_id>/manifest.json` and `game-files/<app_id>/files/...` in its folder, which
 * is Dropbox's App folder, the Nextcloud account's WebDAV files root, or My Drive under the
 * drive.file scope (only files the app created or was given; duplicate names in a path fail,
 * and a file Drive hides from the app is "not_yet_uploaded" with a message saying Drive may be
 * hiding it). The calls and answers are the same for every service. Needs the sign-in or setup
 * that service uses.
 *
 * FOLDERS. `dest_dir` is where the files go and `state_dir` is where the library keeps its
 * checkpoints and `.part` files: both app-private, on the same volume, neither inside the
 * other. Plan, download and the compatibility download refuse (destination_conflict, before any
 * request) a `state_dir` that is, holds or sits inside a saves folder of a game the library
 * remembers (the records save_relay_push and save_relay_pull keep), and a `dest_dir` that is or
 * sits inside one. A `dest_dir` that holds a saves folder (a port keeps its saves in
 * `<game>/user`) is accepted, but the call is refused (destination_conflict, after the manifest
 * is read and before any write) when a manifest path lands in or above that saves folder. A
 * record that cannot be read refuses the call the same way. A saves folder the library was
 * never told about cannot be checked: never pass one.
 *
 * THREADING. Every function may block. save_relay_game_files_download blocks until the download
 * finishes, fails or is cancelled (minutes to hours), and plan, download and the compatibility
 * download make network requests through the `http` callback: call them from a background
 * thread (a foreground service on Android, a background queue or task on iOS), never from the
 * UI thread. save_relay_game_files_progress and save_relay_game_files_cancel return at once and
 * are meant to be called from another thread while a download runs. Every function may be called
 * from any thread. The library never creates threads and never schedules work: the app owns
 * foreground and background execution, and decides when to call download again after the OS
 * stopped it.
 *
 * ONE DOWNLOAD AT A TIME PER PROCESS. A second download (either download function) while one
 * runs answers error code "busy" (class "retryable") and leaves the running one untouched.
 * While a download runs, save_relay_game_files_cleanup and save_relay_init also answer "busy".
 *
 * HOST LIFETIME. The `http` and credential callbacks, and the `user_data` they get, must stay
 * valid until every call that started before the host was replaced has returned: a download
 * (cancel it, wait for it to return), but also plan, push, pull, status and sign-in, which keep
 * using the host they started with. Only then replace or tear down the host. save_relay_init
 * refuses (error code "busy") to replace the host while a download runs, so a running download
 * always uses the host it started with; it does not wait for the other calls. The Kotlin and Swift wrappers follow this rule themselves (see their doc comments).
 *
 * CANCEL. save_relay_game_files_cancel asks the running download to stop. The request is
 * remembered only for the run that is running when it arrives (data.running is true); the run
 * stops after the chunk in flight is durable (a wait to retry is cut short by the cancel, and
 * no source sleeps inside a request), and its download call returns state "cancelled". A later
 * download continues where it stopped. A
 * cancel while nothing runs answers data.running false and changes nothing: it is not kept for
 * the next run. A host that cancels a download it has just started on another thread, before
 * that thread has entered save_relay_game_files_download, must remember the user's cancel itself
 * (do not call download), or cancel again once save_relay_game_files_progress reports the run.
 *
 * PROGRESS. save_relay_game_files_progress answers data: the latest progress of the running or
 * last download (state, app_id, files_done, files_total, bytes_done, bytes_total, current), or
 * null before any download. A download sets its own progress to state "running" for its app as
 * soon as it has started, never the previous run's; it always ends in a terminal state:
 * "complete", "cancelled" or "error" (an error envelope came back from the download call).
 * Poll it about twice a second. Only a call refused because another download
 * holds the slot in this process ("busy"), or with a null or invalid-UTF-8 argument, leaves it
 * unchanged. A call that fails after it took the slot (an invalid app_id, bad folders, a state
 * folder another process holds, not set up, a panic) shows its own app_id with state "error".
 *
 * ERRORS. An error envelope of a game-files command has error.class: "retryable" (network
 * trouble, 429/5xx, "busy"; try again, later), "not_yet_uploaded" (the manifest or a file is not
 * in the service folder yet), "reauth_needed" (sign in again) or "fatal" (do not retry
 * unchanged). error.code is stable within a version: "busy", "not_set_up", "usage",
 * "not_yet_uploaded", "range_unsupported", "remote_changed", "integrity_failed",
 * "destination_conflict", "manifest_invalid", "io" and the usual network and sign-in codes.
 * "remote_changed" is one code with two classes, so decide by class: a file that changed on the
 * service mid-download is "retryable" (the next call restarts that file); a total size that
 * differs from the manifest is "fatal".
 *
 * DURABILITY. A killed process, a dead battery or a dropped network is designed to lose at most
 * one chunk: every chunk is fsynced before its checkpoint. That is tested with a local server and
 * an abort of the process, and is not yet verified on a device, under an OS kill or a power
 * loss. Call download again at the next launch; it continues. Cleanup is the only call that
 * discards partial files.
 *
 * HTTP CALLBACK. Every request asks for a bounded range: the `http` callback must send the Range
 * and If-Match headers, return the 206 answer with its Content-Range, ETag and
 * Dropbox-API-Result headers untouched, send no Accept-Encoding but identity, and read at most
 * max_response_bytes + 1 bytes, handing them over (a longer answer is refused as too large:
 * "range_unsupported" for a range, "manifest_invalid" for the manifest, never retried). It holds
 * one chunk in memory (4 MiB by default), never a file.
 * ------------------------------------------------------------------------------------------- */

/* The version of the game-files interface above: 1. Not an envelope; no allocation. */
int save_relay_game_files_version(void);

/* What a download would do, from file sizes only; changes nothing. data: files and bytes
 * present, partial and missing, and the files in the way. Blocks for one request. */
char *save_relay_game_files_plan(const char *app_id, const char *dest_dir, const char *state_dir);

/* Download or resume everything missing. Blocks until it finishes, fails or is cancelled, so
 * call it from a background thread. data: the final progress, state "complete" or "cancelled".
 * One download runs at a time per process (error code "busy"). */
char *save_relay_game_files_download(const char *app_id, const char *dest_dir,
                                     const char *state_dir);

/* data: the latest progress of the running or last download (state, app_id, files_done,
 * files_total, bytes_done, bytes_total, current), or null before any. Cheap; poll it twice a
 * second. */
char *save_relay_game_files_progress(void);

/* Stop the running download after its current chunk is durable; the download call returns
 * "cancelled" and a later download continues. data.running says whether a download was running
 * and received the request; it is false (and nothing is remembered) when none was. */
char *save_relay_game_files_cancel(void);

/* Remove the checkpoints and `.part` files in `state_dir` (never the game folder), so the next
 * download starts unfinished files again. Refused (error code "busy") while a download runs. */
char *save_relay_game_files_cleanup(const char *state_dir);

/* Compatibility wrappers (#324), kept with the same signatures and envelope commands
 * ("game-files", "game-files-status"); they run the same engine as the functions above.
 * `manifest_json` is `{"version":1,"game_id":...,"remote_root":...,"files":[{path,size,sha256}]}`:
 * the files live at `game-files/<game_id>/<path>` (or under `remote_root`, a plain relative
 * path). `dest_dir` is the game folder; the library keeps its checkpoints in
 * `<dest_dir>/.save-relay-game-files/`. A file already in `dest_dir` that does not match the
 * manifest is a conflict and is never overwritten. New code should use the functions above.
 * Behaviour that differs from before #309: an invalid manifest answers "manifest_invalid"
 * (class "fatal") where it answered "usage"; `game_id` must be a lowercase app id
 * ([a-z0-9][a-z0-9._-]*, at most 128 bytes); a file already present that differs is never
 * overwritten; `.part` files are ignored by status; and `remote_root` is a plain relative path.
 *
 * save_relay_download_game_files follows the same rules as save_relay_game_files_download:
 * background thread, one download at a time per process ("busy"), the host must outlive it. It
 * is stopped by save_relay_game_files_cancel like the others (the call then returns with
 * complete false; a later call continues) and reports no progress;
 * save_relay_game_files_status reads the folder only, offline, and may be called from any
 * thread. */
char *save_relay_download_game_files(const char *manifest_json, const char *dest_dir);
char *save_relay_game_files_status(const char *manifest_json, const char *dest_dir);

/* Free a string returned above. NULL is ignored. */
void save_relay_free(char *pointer);

#ifdef __cplusplus
}
#endif

#endif /* SAVE_RELAY_H */
