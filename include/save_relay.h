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

/* Free a string returned above. NULL is ignored. */
void save_relay_free(char *pointer);

#ifdef __cplusplus
}
#endif

#endif /* SAVE_RELAY_H */
