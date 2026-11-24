package org.saverelay

import java.nio.ByteBuffer
import java.nio.ByteOrder

interface SaveRelayHost {
    fun credentialGet(account: String): ByteArray?
    fun credentialSet(account: String, secret: String): Boolean
    fun credentialDelete(account: String): Boolean
    /**
     * Performs one HTTP request and returns [SaveRelayResponse.toWire], or `null` on a transport
     * failure (DNS, TLS, connection, timeout, or response over [maxResponseBytes]).
     *
     * Android `HttpURLConnection` notes (see `docs/android.md`):
     * - For `POST` with no `Content-Type` header, set `Content-Type: application/octet-stream`.
     * - Only call `setDoOutput(true)` when [body] is non-empty.
     * - Default `Accept-Encoding` to `identity` so response bytes are not transparently unzipped.
     * - Catch `Throwable` and log class, message, method, and queryless URL (never headers/bodies).
     */
    fun http(
        method: String,
        url: String,
        headers: String?,
        body: ByteArray,
        maxResponseBytes: Long,
    ): ByteArray?
}

class SaveRelayResponse(
    @JvmField val status: Int,
    @JvmField val headers: String? = null,
    @JvmField val body: ByteArray = ByteArray(0),
) {
    fun toWire(): ByteArray {
        val headerBytes = (headers ?: "").toByteArray(Charsets.UTF_8)
        val wire = ByteBuffer.allocate(8 + headerBytes.size + body.size)
            .order(ByteOrder.BIG_ENDIAN)
        wire.putInt(status)
        wire.putInt(headerBytes.size)
        wire.put(headerBytes)
        wire.put(body)
        return wire.array()
    }
}
