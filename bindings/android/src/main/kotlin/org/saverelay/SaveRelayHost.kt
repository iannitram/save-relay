package org.saverelay

import java.nio.ByteBuffer
import java.nio.ByteOrder

interface SaveRelayHost {
    fun credentialGet(account: String): ByteArray?
    fun credentialSet(account: String, secret: String): Boolean
    fun credentialDelete(account: String): Boolean
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
