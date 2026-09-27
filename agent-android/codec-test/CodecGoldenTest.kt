package dev.mobileagent.agent.codec

import java.io.File
import kotlin.system.exitProcess

/**
 * Golden cross-check between the agent's codec and crates/protocol, run by
 * agent-android/codec-test.sh on the host JVM and on a device through
 * app_process:
 *
 *     CodecGoldenTestKt --golden DIR [--write DIR]
 *
 * For every sample in [Samples.ALL] it checks that the Kotlin encoder writes
 * the frame the Rust crate wrote to `DIR/<name>.bin` byte for byte, and that
 * decoding that frame and encoding the result gives the same bytes back. With
 * `--write` it also writes the Kotlin frames there for the Rust side to
 * decode. Then it checks the decoder's rules on hand-made payloads.
 *
 * Prints one PASS or FAIL line per check and exits 1 if any failed.
 */
private var passed = 0
private var failed = 0

private fun check(name: String, body: () -> String?) {
    val problem = try {
        body()
    } catch (t: Throwable) {
        t.toString()
    }
    if (problem == null) {
        passed++
        println("PASS $name")
    } else {
        failed++
        println("FAIL $name: $problem")
    }
}

private fun hex(bytes: ByteArray): String = bytes.joinToString(" ") { "%02x".format(it.toInt() and 0xFF) }

private fun bytes(hex: String): ByteArray =
    hex.split(' ').filter { it.isNotEmpty() }.map { it.toInt(16).toByte() }.toByteArray()

/** Null when equal, else both byte strings. */
private fun diff(expected: ByteArray, actual: ByteArray): String? =
    if (expected.contentEquals(actual)) null else "expected ${hex(expected)}\n  actual   ${hex(actual)}"

/** The frame of `msg` with no size limit. */
private fun frame(msg: Message): ByteArray {
    val out = CborWriter()
    Frames.write(msg, -1, out)
    return out.toByteArray()
}

/** Null if decoding `payload` throws a CodecException, else what it decoded. */
private fun rejects(payload: ByteArray): String? = try {
    "decoded ${Codec.decode(payload, 0, payload.size)}"
} catch (e: CodecException) {
    null
}

private fun chain(depth: Int): Node {
    var node = Node(0, 0, null, null, "x", null, Bounds(0, 0, 1, 1), 0, Node.NO_CHILDREN)
    repeat(depth) {
        node = Node(0, 0, null, null, "x", null, Bounds(0, 0, 1, 1), 0, arrayOf(node))
    }
    return node
}

private fun deepSnapshot(depth: Int): ByteArray {
    val window = Window(1, WindowType.APPLICATION, 0, null, null, Bounds(0, 0, 1, 1), 0, chain(depth))
    val out = CborWriter()
    Codec.encode(Response(1, Method.SNAPSHOT, Snapshot(arrayOf(), arrayOf(window), depth + 1, 0, 0), null), out)
    return out.toByteArray()
}

/** A snapshot response's single window, decoded from `hex`. */
private fun onlyWindow(hex: String): Window {
    val payload = bytes(hex)
    val snapshot = (Codec.decode(payload, 0, payload.size) as Response).result as Snapshot
    return snapshot.windows.single()
}

/**
 * A snapshot response with one system window, modal, owned by string 1 and
 * sending no change events, whose root carries `scroll` (hex); the frame of
 * crates/protocol's `tagged_snapshot`.
 */
private fun taggedFrame(scroll: String): String = listOf(
    "a4 00 03 01 00 02 01 03",
    "a5 00 82 61 56 63 73 79 73 01 81",
    "a9 00 01 01 03 02 00 05 84 00 00 01 01 06 00 07",
    "a5 00 00 01 00 06 84 00 00 01 01 07 00 09",
    scroll,
    "08 f4 09 01 0a f5",
    "02 01 03 00 04 00",
).joinToString(" ")

private fun decoderRules() {
    check("rejects an indefinite-length array") {
        rejects(bytes("9f 04 01 05 06 01 0d ff"))
    }
    check("rejects an indefinite-length map") {
        rejects(bytes("bf 00 05 01 00 ff"))
    }
    check("rejects bytes after the message") {
        rejects(bytes("a2 00 05 01 00 00"))
    }
    check("rejects a truncated message") {
        rejects(bytes("a2 00 06 01 1b ff ff"))
    }
    check("rejects a missing required field") {
        rejects(bytes("a1 00 05")) ?: rejects(bytes("a3 00 03 01 01 02 03"))
    }
    check("reports an unknown kind") {
        try {
            val msg = Codec.decode(bytes("a1 00 09"), 0, 3)
            "decoded $msg"
        } catch (e: UnknownKindException) {
            if (e.kind == 9) null else "kind ${e.kind}"
        }
    }
    check("skips unknown map keys") {
        val msg = Codec.decode(bytes("a3 00 05 01 07 09 61 78"), 0, 8)
        if (msg is Ping && msg.nonce == 7L) null else "decoded $msg"
    }
    check("skips trailing event elements") {
        val payload = bytes("87 04 01 05 06 01 0d 61 78")
        val msg = Codec.decode(payload, 0, payload.size)
        val p = (msg as Event).payload
        if (msg.seq == 5L && p is WindowChange && p.change == WindowChangeKind.ADDED && p.window == 13) null else "decoded $msg"
    }
    check("reads null as an absent optional field") {
        // {0: 2, 1: 5, 2: 1, 3: null, 4: {0: null}}: a snapshot request.
        val payload = bytes("a5 00 02 01 05 02 01 03 f6 04 a1 00 f6")
        val msg = Codec.decode(payload, 0, payload.size) as Request
        val call = msg.call as SnapshotCall
        if (msg.deadlineMs == null && call.maxDepth == null) null else "decoded $msg"
    }
    check("accepts nodes nested ${Protocol.MAX_NODE_DEPTH} deep") {
        val payload = deepSnapshot(Protocol.MAX_NODE_DEPTH)
        Codec.decode(payload, 0, payload.size)
        null
    }
    check("rejects nodes nested deeper than ${Protocol.MAX_NODE_DEPTH}") {
        rejects(deepSnapshot(Protocol.MAX_NODE_DEPTH + 1))
    }
    check("reads window metadata and skips what it does not know in scroll semantics") {
        // scroll {0: 1, 1: 2, 2: [0, 0, 199, "x"], 3: [0, 15, 7], 5: "y"}
        val w = onlyWindow(taggedFrame("a5 00 01 01 02 02 84 00 00 18 c7 61 78 03 83 00 0f 07 05 61 79"))
        val scroll = w.root?.scroll
        val range = scroll?.range
        val position = scroll?.position
        when {
            w.emitsEvents != false || w.ownerIndex != 1 || !w.modal ->
                "metadata ${w.emitsEvents} ${w.ownerIndex} ${w.modal}"
            scroll == null || range == null || position == null -> "no scroll semantics"
            scroll.axis != ScrollAxis.VERTICAL || scroll.flags != ScrollFlag.FORWARD -> "axis ${scroll.axis} flags ${scroll.flags}"
            range.unit != ScrollUnit.ITEMS || range.min != 0 || range.max != 199 -> "range ${range.unit} ${range.min} ${range.max}"
            position.first != 0 || position.last != 15 -> "position ${position.first} ${position.last}"
            else -> null
        }
    }
    check("rejects a scroll range or position missing an element, and scroll semantics without flags") {
        rejects(bytes(taggedFrame("a4 00 01 01 02 02 82 00 00 03 82 00 0f")))
            ?: rejects(bytes(taggedFrame("a4 00 01 01 02 02 83 00 00 18 c7 03 81 00")))
            ?: rejects(bytes(taggedFrame("a1 00 01")))
    }
    check("reads a snapshot without the metadata keys with them absent") {
        val old = "a4 00 03 01 00 02 01 03 a5 00 81 61 56 01 81 a6 00 01 01 01 02 00 05 84 00 00 01 01 06 00 07 " +
            "a4 00 00 01 00 06 84 00 00 01 01 07 00 02 01 03 00 04 00"
        val w = onlyWindow(old)
        val payload = bytes(old)
        val out = CborWriter()
        Codec.encode(Codec.decode(payload, 0, payload.size), out)
        if (w.emitsEvents != null || w.ownerIndex != null || w.modal || w.root?.scroll != null) {
            "metadata ${w.emitsEvents} ${w.ownerIndex} ${w.modal} ${w.root?.scroll}"
        } else {
            diff(payload, out.toByteArray())
        }
    }
    check("reads null as absent window metadata and scroll semantics") {
        // window {..., 7: {..., 9: null}, 8: null, 9: null}
        val w = onlyWindow(
            "a4 00 03 01 00 02 01 03 a5 00 81 61 56 01 81 a8 00 01 01 01 02 00 05 84 00 00 01 01 06 00 07 " +
                "a5 00 00 01 00 06 84 00 00 01 01 07 00 09 f6 08 f6 09 f6 02 01 03 00 04 00",
        )
        if (w.emitsEvents == null && w.ownerIndex == null && w.root?.scroll == null) null else "decoded metadata"
    }
    check("reads a probed node and skips the elements after its bounds") {
        // A bounds response whose one node, 7, carries an element after its bounds.
        val payload = bytes("a4 00 03 01 01 02 0b 03 a1 00 81 83 07 84 00 20 18 18 19 09 60 61 78")
        val probed = (Codec.decode(payload, 0, payload.size) as Response).result as Probed
        val node = probed.nodes.single()
        val b = node.bounds
        when {
            node.node != 7 -> "node ${node.node}"
            b.left != 0 || b.top != -1 || b.right != 24 || b.bottom != 2400 ->
                "bounds ${b.left} ${b.top} ${b.right} ${b.bottom}"
            else -> null
        }
    }
    check("rejects a probed node without its bounds") {
        rejects(bytes("a4 00 03 01 01 02 0b 03 a1 00 81 81 07"))
    }
    check("rejects an array longer than its input without allocating it") {
        rejects(bytes("87 04 00 01 01 00 01 9a ff ff ff ff"))
    }
    check("rejects invalid UTF-8") {
        // A lifecycle event whose message is the lone byte 0xff.
        rejects(bytes("86 04 03 01 01 01 61 ff"))
    }
    check("rejects a request id beyond 32 bits") {
        rejects(bytes("a4 00 02 01 1b 00 00 00 01 00 00 00 00 02 01 04 a0"))
    }
    check("refuses to write a frame over the limit and writes nothing") {
        val out = CborWriter()
        out.writeByte(0x42)
        val msg = Response(2, Method.SCREENSHOT, Screenshot(ImageFormat.PNG, 1, 1, 0, ByteArray(5000)), null)
        try {
            Frames.write(msg, 4096, out)
            "wrote ${out.pos} bytes"
        } catch (e: FrameTooLargeException) {
            when {
                out.pos != 1 -> "left ${out.pos - 1} bytes behind"
                e.limit != 4096L -> "limit ${e.limit}"
                "4096" !in e.message.orEmpty() -> "message ${e.message}"
                else -> null
            }
        }
    }
    check("reads the frame length as unsigned") {
        val header = bytes("ff ff ff ff")
        val len = Frames.payloadLength(header, 0)
        if (len == 0xFFFFFFFFL) null else "read $len"
    }
}

fun main(args: Array<String>) {
    val opts = args.toList().chunked(2).associate { it[0] to it.getOrElse(1) { "" } }
    val golden = File(opts["--golden"] ?: usage())
    val write = opts["--write"]?.let { File(it) }
    write?.mkdirs()

    val names = Samples.ALL.map { it.first }.toSortedSet()
    check("the golden directory holds exactly the samples") {
        val files = golden.listFiles()?.filter { it.isFile && it.name.endsWith(".bin") }
            ?.map { it.name.removeSuffix(".bin") }?.toSortedSet()
        if (files == names) null else "files $files, samples $names"
    }
    for ((name, msg) in Samples.ALL) {
        val file = File(golden, "$name.bin")
        if (!file.isFile) {
            check("$name: golden frame") { "missing ${file.path}" }
            continue
        }
        val rust = file.readBytes()
        check("$name: the Kotlin encoder writes the Rust frame") {
            diff(rust, frame(msg))
        }
        check("$name: the Rust frame decodes and encodes back to itself") {
            val len = Frames.payloadLength(rust, 0)
            if (len != (rust.size - Frames.HEADER_LEN).toLong()) {
                "header says $len bytes, the file holds ${rust.size - Frames.HEADER_LEN}"
            } else {
                diff(rust, frame(Codec.decode(rust, Frames.HEADER_LEN, rust.size - Frames.HEADER_LEN)))
            }
        }
        write?.let { File(it, "$name.bin").writeBytes(frame(msg)) }
    }
    decoderRules()
    println("codec: $passed passed, $failed failed")
    exitProcess(if (failed == 0) 0 else 1)
}

private fun usage(): Nothing {
    System.err.println("usage: CodecGoldenTestKt --golden DIR [--write DIR]")
    exitProcess(2)
}
