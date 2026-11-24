package org.saverelay

class SaveRelay(
    home: String,
    data: String,
    config: String,
    documents: String? = null,
    host: SaveRelayHost,
) : AutoCloseable {
    val initialized: String = take(nativeInit(home, data, config, documents, host))

    private var closed = false

    fun pull(manifestJson: String?, savesDir: String?, resolve: String? = null): String =
        take(nativePull(manifestJson, savesDir, resolve))

    fun push(manifestJson: String?, savesDir: String?, resolve: String? = null): String =
        take(nativePush(manifestJson, savesDir, resolve))

    fun status(manifestJson: String?, savesDir: String?): String =
        take(nativeStatus(manifestJson, savesDir))

    fun setup(
        server: String?,
        user: String?,
        password: String?,
        device: String? = null,
        root: String? = null,
    ): String = take(nativeSetup(server, user, password, device, root))

    fun signInStart(
        service: String?,
        redirectUri: String?,
        device: String? = null,
        root: String? = null,
    ): String = take(nativeSignInStart(service, redirectUri, device, root))

    /**
     * Completes browser OAuth using the redirect URL returned to the app.
     *
     * On Android 15+, reply to the loopback redirect and bring the app back to the foreground
     * before calling this method (backgrounded apps without a foreground service have outbound
     * DNS/HTTP blocked). On `server_unreachable` or `server_error`, save-relay keeps the pending
     * sign-in so the caller can retry briefly once foreground network access is restored.
     * See `docs/android.md`.
     */
    fun signInFinish(redirectUrl: String?): String =
        take(nativeSignInFinish(redirectUrl))

    // Game files: resumable downloads from Dropbox (docs/assets.md, #305). `destDir` is where an
    // app's files go and `stateDir` where the library keeps its checkpoints and `.part` files:
    // app-private, on the same file system, neither inside the other, neither the saves folder.
    // Every answer is the usual JSON envelope; an error also carries `error.class`
    // ("retryable", "not_yet_uploaded", "reauth_needed" or "fatal").

    /** What a download would do, from file sizes only (nothing is written). */
    fun gameFilesPlan(appId: String?, destDir: String?, stateDir: String?): String =
        take(nativeGameFilesPlan(appId, destDir, stateDir))

    /**
     * Downloads or resumes everything missing. Blocks until it finishes, fails or is cancelled:
     * run it on a background thread (a foreground service for gigabytes) and poll
     * [gameFilesProgress] from another. A killed process or a dropped network loses at most one
     * chunk; call it again and it continues. The host's `SaveRelayHost.http` must send the
     * request's `Range` header, return the 206 answer with its `Content-Range` and
     * `Dropbox-API-Result` headers unchanged, use `Accept-Encoding: identity` and
     * `Content-Type: application/octet-stream` (a bodiless POST otherwise gets a form type and a
     * 400 from Dropbox), and stop at `maxResponseBytes`.
     */
    fun gameFilesDownload(appId: String?, destDir: String?, stateDir: String?): String =
        take(nativeGameFilesDownload(appId, destDir, stateDir))

    /** The latest progress of the running or last download (`data` is null before any). */
    fun gameFilesProgress(): String = take(nativeGameFilesProgress())

    /** Stops the running download after its current chunk is durable. */
    fun gameFilesCancel(): String = take(nativeGameFilesCancel())

    /** Removes the checkpoints and `.part` files in [stateDir] (never the game folder). */
    fun gameFilesCleanup(stateDir: String?): String = take(nativeGameFilesCleanup(stateDir))

    override fun close() {
        if (!closed) {
            closed = true
            nativeShutdown()
        }
    }

    private fun take(pointer: Long): String {
        if (pointer == 0L) return ""
        return try {
            nativeRead(pointer)
        } finally {
            nativeFree(pointer)
        }
    }

    private external fun nativeInit(
        home: String,
        data: String,
        config: String,
        documents: String?,
        host: SaveRelayHost,
    ): Long

    private external fun nativeRead(pointer: Long): String

    private external fun nativeFree(pointer: Long)

    private external fun nativeShutdown()

    private external fun nativePull(
        manifestJson: String?,
        savesDir: String?,
        resolve: String?,
    ): Long

    private external fun nativePush(
        manifestJson: String?,
        savesDir: String?,
        resolve: String?,
    ): Long

    private external fun nativeStatus(manifestJson: String?, savesDir: String?): Long

    private external fun nativeSetup(
        server: String?,
        user: String?,
        password: String?,
        device: String?,
        root: String?,
    ): Long

    private external fun nativeSignInStart(
        service: String?,
        redirectUri: String?,
        device: String?,
        root: String?,
    ): Long

    private external fun nativeSignInFinish(redirectUrl: String?): Long

    private external fun nativeGameFilesPlan(
        appId: String?,
        destDir: String?,
        stateDir: String?,
    ): Long

    private external fun nativeGameFilesDownload(
        appId: String?,
        destDir: String?,
        stateDir: String?,
    ): Long

    private external fun nativeGameFilesProgress(): Long

    private external fun nativeGameFilesCancel(): Long

    private external fun nativeGameFilesCleanup(stateDir: String?): Long

    companion object {
        init {
            System.loadLibrary("save_relay")
        }
    }
}
