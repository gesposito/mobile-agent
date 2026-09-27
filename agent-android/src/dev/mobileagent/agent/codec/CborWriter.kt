package dev.mobileagent.agent.codec

/**
 * A growable buffer that CBOR (RFC 8949) is written into in preferred
 * serialization: every head in its shortest form and every length definite,
 * as minicbor writes on the host, so both sides produce the same bytes.
 * Unsigned 32-bit values are passed as Ints and 64-bit ones as Longs holding
 * the bit pattern. Reuse one writer with [reset] to avoid allocating per frame.
 */
class CborWriter(initialCapacity: Int = 256) {
    @JvmField var buf: ByteArray = ByteArray(initialCapacity.coerceAtLeast(16))
    @JvmField var pos: Int = 0

    fun reset() {
        pos = 0
    }

    fun toByteArray(): ByteArray = buf.copyOf(pos)

    /** Makes room for `extra` more bytes. */
    fun ensure(extra: Int) {
        val need = pos + extra
        if (need < 0) throw IllegalStateException("a CBOR buffer cannot exceed 2 GiB")
        if (need > buf.size) {
            val doubled = if (buf.size > Int.MAX_VALUE / 2) Int.MAX_VALUE else buf.size * 2
            buf = buf.copyOf(maxOf(need, doubled))
        }
    }

    fun writeByte(b: Int) {
        ensure(1)
        buf[pos++] = b.toByte()
    }

    /** A head of `major` type whose argument is the unsigned 64-bit bit pattern `value`. */
    fun head(major: Int, value: Long) {
        ensure(9)
        val b = buf
        var p = pos
        val m = major shl 5
        if (value in 0L..23L) {
            b[p++] = (m or value.toInt()).toByte()
        } else if (value in 24L..0xFFL) {
            b[p++] = (m or 24).toByte()
            b[p++] = value.toByte()
        } else if (value in 0x100L..0xFFFFL) {
            b[p++] = (m or 25).toByte()
            b[p++] = (value shr 8).toByte()
            b[p++] = value.toByte()
        } else if (value in 0x10000L..0xFFFFFFFFL) {
            b[p++] = (m or 26).toByte()
            for (shift in 24 downTo 0 step 8) b[p++] = (value shr shift).toByte()
        } else {
            // 2^32 and up, including the bit patterns of 2^63 and up, which are negative Longs.
            b[p++] = (m or 27).toByte()
            for (shift in 56 downTo 0 step 8) b[p++] = (value shr shift).toByte()
        }
        pos = p
    }

    /** An unsigned 32-bit value held in an Int. */
    fun uint(value: Int) = head(0, value.toLong() and 0xFFFFFFFFL)

    /** An unsigned 64-bit value held in a Long. */
    fun ulong(value: Long) = head(0, value)

    /** A signed 32-bit value. */
    fun sint(value: Int) = if (value >= 0) head(0, value.toLong()) else head(1, -1L - value)

    fun bool(value: Boolean) = writeByte(if (value) 0xF5 else 0xF4)

    fun array(size: Int) = head(4, size.toLong())

    fun map(size: Int) = head(5, size.toLong())

    /** A map key: a field index. */
    fun key(index: Int) = head(0, index.toLong())

    fun bytes(value: ByteArray) {
        head(2, value.size.toLong())
        ensure(value.size)
        System.arraycopy(value, 0, buf, pos, value.size)
        pos += value.size
    }

    /** A text string, written as UTF-8 without an intermediate array. */
    fun text(value: String) {
        head(3, utf8Length(value).toLong())
        writeUtf8(value)
    }

    private fun writeUtf8(s: String) {
        val n = s.length
        ensure(n * 3)
        val b = buf
        var p = pos
        var i = 0
        while (i < n) {
            val c = s[i].code
            if (c < 0x80) {
                b[p++] = c.toByte()
            } else if (c < 0x800) {
                b[p++] = (0xC0 or (c shr 6)).toByte()
                b[p++] = (0x80 or (c and 0x3F)).toByte()
            } else if (c in 0xD800..0xDBFF && i + 1 < n && s[i + 1].code in 0xDC00..0xDFFF) {
                val cp = 0x10000 + ((c - 0xD800) shl 10) + (s[i + 1].code - 0xDC00)
                b[p++] = (0xF0 or (cp shr 18)).toByte()
                b[p++] = (0x80 or ((cp shr 12) and 0x3F)).toByte()
                b[p++] = (0x80 or ((cp shr 6) and 0x3F)).toByte()
                b[p++] = (0x80 or (cp and 0x3F)).toByte()
                i++
            } else {
                // An unpaired surrogate becomes U+FFFD, as String.getBytes does.
                val cp = if (c in 0xD800..0xDFFF) 0xFFFD else c
                b[p++] = (0xE0 or (cp shr 12)).toByte()
                b[p++] = (0x80 or ((cp shr 6) and 0x3F)).toByte()
                b[p++] = (0x80 or (cp and 0x3F)).toByte()
            }
            i++
        }
        pos = p
    }

    companion object {
        /** The UTF-8 length of `s` as [text] writes it. */
        @JvmStatic
        fun utf8Length(s: String): Int {
            val n = s.length
            var len = n
            var i = 0
            while (i < n) {
                val c = s[i].code
                if (c >= 0x80) {
                    if (c < 0x800) {
                        len += 1
                    } else if (c in 0xD800..0xDBFF && i + 1 < n && s[i + 1].code in 0xDC00..0xDFFF) {
                        len += 2
                        i++
                    } else {
                        len += 2
                    }
                }
                i++
            }
            return len
        }
    }
}
