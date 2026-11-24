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

    companion object {
        init {
            System.loadLibrary("save_relay")
        }
    }
}
