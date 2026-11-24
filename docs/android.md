# Android integration notes

Android apps link `libsave_relay.a` (built with `--no-default-features --features ffi`), compile `bindings/android/src/main/cpp/save_relay_jni.c`, and call it through `org.saverelay.SaveRelay` and `org.saverelay.SaveRelayHost` (`bindings/android/src/main/kotlin/org/saverelay/`).

This note records platform rules verified on Android 15 (API 35) devices so every host app avoids the same pitfalls.

## Browser OAuth (`signInStart` / `signInFinish`) on Android 15+

Dropbox and Google Drive sign-in use PKCE in the system browser, returning to a loopback address (for Dropbox, `http://127.0.0.1:53710/`) served by a `ServerSocket` in the app.

**Do not call `signInFinish(redirectUrl)` inside the loopback HTTP handler while the browser is still in the foreground.**

- ~5 seconds after the browser takes the foreground, Android 15's network policy marks an app without an active foreground service as `blocked=APP_BACKGROUND` (`dumpsys connectivity`).
- Loopback (`127.0.0.1`) still reaches the app's `ServerSocket`, so the OAuth redirect arrives while the app is still backgrounded.
- Outbound DNS/HTTPS from the backgrounded app is blocked by Android's UID firewall, causing `POST https://api.dropboxapi.com/oauth2/token` to fail with `UnknownHostException: Unable to resolve host "api.dropboxapi.com": No address associated with hostname`.

### Required flow

1. **Return to the app first.** When the loopback handler receives the matching `state` and `code`, immediately reply with an HTML page that redirects back to the app via an `intent://` URI (plus a manual "Back to app" button in case the browser blocks auto-redirect) and close the client socket.
2. **Reuse the host Activity.** Mark the Activity that handles the return `intent://` scheme with `android:launchMode="singleTask"` in `AndroidManifest.xml` so the browser brings the existing Activity back to the front instead of spawning a second instance on top of the browser's task.
3. **Wait for foreground (or a foreground service).** Track `onResume()` / `onPause()` on the host Activity. Wait until the Activity is foregrounded (or a foreground service is running) before calling `signInFinish(redirectUrl)` on a worker thread.
4. **Retry briefly on network errors.** `netd` lifts the UID background firewall rule as the Activity resumes; if the first `signInFinish` call races it, sleep briefly (e.g., 300 ms) and retry up to a few times. `save_relay_sign_in_finish` preserves the pending sign-in state on `server_unreachable` and `server_error` specifically so retries with the same `redirectUrl` work without restarting OAuth.

## `SaveRelayHost.http` with `HttpURLConnection`

- **Dropbox bodyless `POST` requests:** Android's `HttpURLConnection` sets `Content-Type: application/x-www-form-urlencoded` on a `POST` that has no `Content-Type` header, which Dropbox's download and list RPCs reject with HTTP 400 (their arguments travel in `Dropbox-API-Arg`). When `method` is `POST` and the caller did not supply `Content-Type`, set `Content-Type: application/octet-stream`.
- **Empty request bodies:** Only call `setDoOutput(true)` and `setFixedLengthStreamingMode(body.size)` when `body.isNotEmpty()`.
- **No transparent gzip:** If the caller did not set `Accept-Encoding`, set `Accept-Encoding: identity` so response bytes and lengths match what the server sent.
- **Diagnostics without leaking secrets:** Catch `Throwable` (not only `Exception`) and log the exception class, message, HTTP method, and URL with its query string stripped (never headers, bodies, or tokens). Returning `null` without logging reduces every failure to `"the app's network call failed"`.
