package dev.mobileagent.agent.codec

/**
 * Framing: every message travels as a 32-bit big-endian payload length
 * followed by exactly one CBOR item of that length. The limit applies to the
 * payload; before the handshake it is this side's own maximum, afterwards the
 * negotiated one.
 */
object Frames {
    const val HEADER_LEN = 4

    /**
     * Appends `msg` to `out` as one frame. Throws [FrameTooLargeException],
     * leaving `out` as it was, when the payload exceeds `limit`, an unsigned
     * 32-bit value held in an Int (-1 for no practical limit). An agent
     * answers the request with a frame_too_large error instead.
     */
    fun write(msg: Message, limit: Int, out: CborWriter) {
        val start = out.pos
        out.ensure(HEADER_LEN)
        out.pos += HEADER_LEN
        Codec.encode(msg, out)
        val size = (out.pos - start - HEADER_LEN).toLong()
        val max = limit.toLong() and 0xFFFFFFFFL
        if (size > max) {
            out.pos = start
            throw FrameTooLargeException(size, max)
        }
        val b = out.buf
        b[start] = (size ushr 24).toByte()
        b[start + 1] = (size ushr 16).toByte()
        b[start + 2] = (size ushr 8).toByte()
        b[start + 3] = size.toByte()
    }

    /** The payload length in the frame header at `buf[off]`, unsigned. */
    fun payloadLength(buf: ByteArray, off: Int): Long {
        if (off < 0 || buf.size - off < HEADER_LEN) throw CodecException("a frame header needs $HEADER_LEN bytes")
        return ((buf[off].toLong() and 0xFF) shl 24) or
            ((buf[off + 1].toLong() and 0xFF) shl 16) or
            ((buf[off + 2].toLong() and 0xFF) shl 8) or
            (buf[off + 3].toLong() and 0xFF)
    }
}

/** A message over the frame limit, refused before any of it was written. */
class FrameTooLargeException(@JvmField val size: Long, @JvmField val limit: Long) :
    RuntimeException("a frame of $size bytes exceeds the maximum frame size of $limit bytes")
