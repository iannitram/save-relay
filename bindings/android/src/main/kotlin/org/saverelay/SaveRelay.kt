package org.saverelay

/**
 * save-relay for Android. Create one instance for the process and keep it (an application-level
 * singleton): the native library holds one host (credentials and network) at a time, and the
 * newest successfully initialised instance owns it.
 *
 * Threading: every method may block on the network or the disk, so call all of them off the main
 * thread. The game-file methods are meant to be used from several threads at once: run
 * [gameFilesDownload] on one background thread (a foreground service for gigabytes) and call
 * [gameFilesProgress] and [gameFilesCancel] from others; they return at once. One download runs
 * at a time per process. See `include/save_relay.h` for the full contract (version 1).
 */
class SaveRelay(
    home: String,
    data: String,
    config: String,
    documents: String? = null,
    host: SaveRelayHost,
) : AutoCloseable {
    /**
     * The answer of the native init. The library and this wrapper's own bookkeeping change the
     * current host under one lock, so several instances created at once agree with the library
     * on which one is current.
     */
    val initialized: String =
        synchronized(registry) {
            take(nativeInit(home, data, config, documents, host)).also { line ->
                // Only an instance the library accepted owns the host: a refused one (`busy`,
                // while a download runs) must never tear down the running download's host.
                if (line.contains("\"ok\":true")) current = this
            }
        }

    /**
     * Whether the library accepted this instance's host. A refused instance (`busy`, because a
     * download was running) is not connected: its calls would use whichever host is active, so
     * create it again once the download has returned.
     */
    val isReady: Boolean = initialized.contains("\"ok\":true")

    @Suppress("PLATFORM_CLASS_MAPPED_TO_KOTLIN")
    private val lifecycle = java.lang.Object()
    private var activeDownloads = 0
    private var closeRequested = false

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

    // Game files: resumable downloads (docs/assets.md, #305, #309, #310). They come from whichever
    // service this device is signed in to or set up for: the Dropbox App folder, the Nextcloud
    // WebDAV files root, or My Drive under `drive.file`, with the same calls and answers for every
    // service. `destDir` is where an app's files go and `stateDir` where the library keeps its
    // checkpoints and `.part` files: app-private, on the same file system, neither inside the
    // other, and neither inside a saves folder (`destDir` may hold one, if no file of the
    // manifest lands in or above it). Every answer is the usual JSON envelope; an error also
    // carries `error.class`
    // ("retryable", "not_yet_uploaded", "reauth_needed" or "fatal"). Contract version
    // [GAME_FILES_VERSION]; the header `include/save_relay.h` is the reference.

    /** The version of the game-file interface the loaded native library speaks (see [GAME_FILES_VERSION]). */
    fun gameFilesVersion(): Int = nativeGameFilesVersion()

    /**
     * What a download would do, from file sizes only (nothing is written). Makes one network
     * request: call it off the main thread.
     */
    fun gameFilesPlan(appId: String?, destDir: String?, stateDir: String?): String =
        take(nativeGameFilesPlan(appId, destDir, stateDir))

    /**
     * Downloads or resumes everything missing. Blocks until it finishes, fails or is cancelled
     * (minutes to hours): run it on a background thread, never the main thread (a foreground
     * service for gigabytes), and poll [gameFilesProgress] from another. One download runs at
     * a time per process; a second call answers error code `busy` (class `retryable`) and
     * leaves the running one alone. A cancel ([gameFilesCancel]) stops it after the current
     * chunk is durable and the answer's `data.state` is `cancelled`; a killed process or a
     * dropped network loses at most one chunk. Call it again and it continues.
     *
     * The host's `SaveRelayHost.http` must send the request's `Range` and `If-Match` headers,
     * return the 206 answer with its `Content-Range`, `ETag` and `Dropbox-API-Result` headers
     * unchanged, use `Accept-Encoding: identity` and `Content-Type: application/octet-stream`
     * (a bodiless POST otherwise gets a form type and a 400 from Dropbox), and stop reading after
     * `maxResponseBytes + 1` bytes, returning them (a longer answer is refused as too large, not
     * retried). A file that changes on the service mid-download is code `remote_changed`
     * (class `retryable`: the next call restarts that file).
     *
     * [close] while this runs cancels it and releases the host only after it has returned.
     */
    fun gameFilesDownload(appId: String?, destDir: String?, stateDir: String?): String =
        trackedDownload("game-files-download") {
            take(nativeGameFilesDownload(appId, destDir, stateDir))
        }

    /**
     * The latest progress of the running or last download (`data` is null before any): `state`
     * (`running`, `complete`, `cancelled` or `error`), `app_id`, `files_done`, `files_total`,
     * `bytes_done`, `bytes_total`, `current`. Returns at once; poll about twice a second from a
     * thread other than the one running the download.
     */
    fun gameFilesProgress(): String = take(nativeGameFilesProgress())

    /**
     * Stops the running download after its current chunk is durable (its [gameFilesDownload]
     * call then returns `cancelled`); a later download continues. `data.running` says whether a
     * download was running. A cancel while none runs is not remembered for the next one: if you
     * cancel a download you have only just handed to another thread, keep your own flag and do
     * not start it, or cancel again once [gameFilesProgress] shows it running. Returns at once.
     */
    fun gameFilesCancel(): String = take(nativeGameFilesCancel())

    /**
     * Removes the checkpoints and `.part` files in [stateDir] (never the game folder). Answers
     * `busy` while a download runs.
     */
    fun gameFilesCleanup(stateDir: String?): String = take(nativeGameFilesCleanup(stateDir))

    /**
     * Compatibility (#324): downloads the files of an inline manifest, through the same engine
     * and with the same rules as [gameFilesDownload] (background thread, one download at a time
     * per process, resumable). [gameFilesCancel] stops it too (it returns with `complete` false); it reports no
     * progress. New code uses
     * [gameFilesDownload].
     */
    fun downloadGameFiles(manifestJson: String?, destDir: String?): String =
        trackedDownload("game-files") { take(nativeDownloadGameFiles(manifestJson, destDir)) }

    /** Compatibility (#324): what [destDir] holds of an inline manifest, read offline. */
    fun gameFilesStatus(manifestJson: String?, destDir: String?): String =
        take(nativeGameFilesStatus(manifestJson, destDir))

    /**
     * Cancels a running download, waits until its call has returned (it is cancelled first, so
     * that is after the current chunk plus, at worst, a retry wait) and then releases the native
     * host. Because it returns only after the host is released, a new [SaveRelay] created right
     * afterwards is accepted. It can therefore block: call it off the main thread whenever a
     * download may be running. Without a running download it returns at once. Idempotent.
     */
    override fun close() {
        synchronized(lifecycle) {
            if (closeRequested) return
            closeRequested = true
            // A cancel that arrives before the download call has claimed the library's run slot
            // is not remembered, so keep asking until the call has returned.
            while (activeDownloads > 0) {
                take(nativeGameFilesCancel())
                lifecycle.wait(CANCEL_REPEAT_MS)
            }
        }
        releaseHost()
    }

    /** Runs a download call, counting it so [close] cannot release the host under it. */
    private fun trackedDownload(command: String, call: () -> String): String {
        synchronized(lifecycle) {
            if (closeRequested) {
                return "{\"ok\":false,\"command\":\"$command\",\"error\":{\"class\":\"fatal\"," +
                    "\"code\":\"usage\",\"message\":\"SaveRelay is closed\"}}"
            }
            activeDownloads++
        }
        try {
            return call()
        } finally {
            synchronized(lifecycle) {
                activeDownloads--
                lifecycle.notifyAll()
            }
        }
    }

    private fun releaseHost() {
        synchronized(registry) {
            if (current === this) {
                current = null
                nativeShutdown()
            }
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

    private external fun nativeGameFilesVersion(): Int

    private external fun nativeDownloadGameFiles(manifestJson: String?, destDir: String?): Long

    private external fun nativeGameFilesStatus(manifestJson: String?, destDir: String?): Long

    companion object {
        /** The game-file interface version this wrapper was written against (`save_relay_game_files_version`). */
        const val GAME_FILES_VERSION = 1

        /** How often [close] repeats its cancel while it waits for a download to return. */
        private const val CANCEL_REPEAT_MS = 200L

        private val registry = Any()
        private var current: SaveRelay? = null

        init {
            System.loadLibrary("save_relay")
        }
    }
}
