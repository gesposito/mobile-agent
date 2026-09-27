package dev.mobileagent.agent.codec

/**
 * A strict CBOR reader for the protocol's messages in `buf[pos, end)`. It
 * accepts definite lengths only, checks every integer against the width of
 * its field, decodes text as strict UTF-8 and throws [CodecException] on
 * anything else, so it refuses what the host's decoder refuses. Every length
 * is checked against the bytes left before anything is allocated for it.
 */
class CborReader(@JvmField val buf: ByteArray, @JvmField var pos: Int, @JvmField val end: Int) {
    /** The major type of the last head read. */
    @JvmField var major: Int = 0

    /** Reads a head and returns its argument as an unsigned 64-bit bit pattern. */
    fun head(): Long {
        if (pos >= end) throw CodecException("the message ends at byte $pos")
        val initial = buf[pos++].toInt() and 0xFF
        major = initial ushr 5
        val info = initial and 0x1F
        return when {
            info < 24 -> info.toLong()
            info <= 27 -> bigEndian(1 shl (info - 24))
            info == 31 -> throw CodecException(
                "indefinite-length item at byte ${pos - 1}; the protocol uses definite lengths only",
            )
            else -> throw CodecException("reserved additional information $info at byte ${pos - 1}")
        }
    }

    /** The major type of the next item, without reading it. */
    fun peekMajor(): Int {
        if (pos >= end) throw CodecException("the message ends at byte $pos")
        return (buf[pos].toInt() and 0xFF) ushr 5
    }

    private fun bigEndian(n: Int): Long {
        if (end - pos < n) throw CodecException("the message ends inside a head at byte $pos")
        var v = 0L
        repeat(n) { v = (v shl 8) or (buf[pos++].toLong() and 0xFF) }
        return v
    }

    private fun expect(m: Int, what: String): Long {
        val at = pos
        val v = head()
        if (major != m) throw CodecException("expected $what at byte $at, found major type $major")
        return v
    }

    private fun unsigned(max: Long, what: String): Long {
        val at = pos
        val v = expect(0, what)
        if (v < 0 || v > max) throw CodecException("${v.toULong()} at byte $at does not fit $what")
        return v
    }

    fun uint8(): Int = unsigned(0xFFL, "a u8").toInt()

    fun uint16(): Int = unsigned(0xFFFFL, "a u16").toInt()

    /** An unsigned 32-bit value, returned as an Int holding its bit pattern. */
    fun uint32(): Int = unsigned(0xFFFFFFFFL, "a u32").toInt()

    /** An unsigned 64-bit value, returned as a Long holding its bit pattern. */
    fun uint64(): Long = expect(0, "an unsigned integer")

    fun int32(): Int {
        val at = pos
        val v = head()
        val value = when (major) {
            0 -> v
            1 -> -1L - v
            else -> throw CodecException("expected an integer at byte $at, found major type $major")
        }
        if (v < 0 || value < Int.MIN_VALUE || value > Int.MAX_VALUE) {
            throw CodecException("the integer at byte $at does not fit an i32")
        }
        return value.toInt()
    }

    /** A map key: any integer that fits an i64, as the host reads keys. */
    fun key(): Long {
        val at = pos
        val v = head()
        if (v < 0 || (major != 0 && major != 1)) throw CodecException("expected an integer key at byte $at")
        return if (major == 0) v else -1L - v
    }

    fun bool(): Boolean {
        if (pos >= end) throw CodecException("the message ends at byte $pos")
        return when (val b = buf[pos++].toInt() and 0xFF) {
            0xF5 -> true
            0xF4 -> false
            else -> throw CodecException("expected a boolean at byte ${pos - 1}, found 0x${b.toString(16)}")
        }
    }

    fun text(): String {
        val n = length(3, "a text string")
        val s = try {
            buf.decodeToString(pos, pos + n, throwOnInvalidSequence = true)
        } catch (e: CharacterCodingException) {
            throw CodecException("invalid UTF-8 in the text string at byte $pos")
        }
        pos += n
        return s
    }

    fun bytes(): ByteArray {
        val n = length(2, "a byte string")
        val out = buf.copyOfRange(pos, pos + n)
        pos += n
        return out
    }

    private fun length(m: Int, what: String): Int {
        val at = pos
        val v = expect(m, what)
        if (v < 0 || v > end - pos) throw CodecException("$what of ${v.toULong()} bytes at byte $at runs past the end")
        return v.toInt()
    }

    /** An array header: its length, which is at most the bytes left. */
    fun array(): Int = count(4, "an array", 1)

    /** A map header: its number of entries, which is at most half the bytes left. */
    fun map(): Int = count(5, "a map", 2)

    private fun count(m: Int, what: String, bytesPerItem: Int): Int {
        val at = pos
        val v = expect(m, what)
        if (v < 0 || v > (end - pos) / bytesPerItem) {
            throw CodecException("$what of ${v.toULong()} items at byte $at is longer than the message")
        }
        return v.toInt()
    }

    /** Whether the next item is null, which stands for an absent optional field. */
    fun nextIsNull(): Boolean = pos < end && (buf[pos].toInt() and 0xFF) == 0xF6

    fun optUint8(): Int? = if (nextIsNull()) { pos++; null } else uint8()

    fun optUint16(): Int? = if (nextIsNull()) { pos++; null } else uint16()

    fun optUint32(): Int? = if (nextIsNull()) { pos++; null } else uint32()

    fun optText(): String? = if (nextIsNull()) { pos++; null } else text()

    /** Skips one item, iteratively, so nesting cannot exhaust the stack. */
    fun skip() {
        var items = 1L
        while (items > 0) {
            items--
            val at = pos
            val v = head()
            when (major) {
                2, 3 -> {
                    if (v < 0 || v > end - pos) throw CodecException("a string at byte $at runs past the end")
                    pos += v.toInt()
                }
                4 -> {
                    if (v < 0 || v > end - pos) throw CodecException("an array at byte $at is longer than the message")
                    items += v
                }
                5 -> {
                    if (v < 0 || v > (end - pos) / 2) throw CodecException("a map at byte $at is longer than the message")
                    items += 2 * v
                }
                6 -> items++
                // 0, 1 and 7: the head was the whole item.
            }
        }
    }

    /** Skips a map, whatever it holds; its keys must still be integers. */
    fun skipMap() {
        repeat(map()) {
            key()
            skip()
        }
    }
}

/** A message that does not follow the protocol's encoding. */
open class CodecException(message: String) : RuntimeException(message)

/** A message kind this version does not know; a session skips these. */
class UnknownKindException(@JvmField val kind: Int) : CodecException("unknown message kind $kind")
