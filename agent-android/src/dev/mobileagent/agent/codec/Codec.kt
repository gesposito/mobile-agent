package dev.mobileagent.agent.codec

/**
 * Encodes and decodes the protocol's messages in the layout crates/protocol
 * uses, so both sides write the same bytes: messages, snapshots, windows,
 * nodes, scroll semantics, parameters, results and errors are maps keyed by
 * field index, keys in ascending order, absent optional fields left out;
 * events, bounds, scroll ranges and scroll positions are positional arrays;
 * every length is definite.
 *
 * Decoding skips unknown map keys and array elements past the ones this
 * version defines, reads null as an absent optional field, lets a repeated
 * key's last value win, and throws [CodecException] for anything the host's
 * decoder would refuse, [UnknownKindException] for a kind it does not know.
 */
object Codec {
    /** Appends `msg` to `out`. */
    fun encode(msg: Message, out: CborWriter) {
        when (msg) {
            is Hello -> encodeHello(msg, out)
            is Welcome -> encodeWelcome(msg, out)
            is Request -> encodeRequest(msg, out)
            is Response -> encodeResponse(msg, out)
            is Event -> encodeEvent(msg, out)
            is Ping -> encodeNonce(Kind.PING, msg.nonce, out)
            is Pong -> encodeNonce(Kind.PONG, msg.nonce, out)
        }
    }

    /** Decodes the one message that fills `buf[off, off + len)`. */
    fun decode(buf: ByteArray, off: Int, len: Int): Message {
        val end = off + len
        val r = CborReader(buf, off, end)
        val msg = when (val kind = peekKind(r)) {
            Kind.HELLO -> decodeHello(r)
            Kind.WELCOME -> decodeWelcome(r)
            Kind.REQUEST -> decodeRequest(r)
            Kind.RESPONSE -> decodeResponse(r)
            Kind.EVENT -> decodeEvent(r)
            Kind.PING -> Ping(decodeNonce(r, Kind.PING))
            Kind.PONG -> Pong(decodeNonce(r, Kind.PONG))
            else -> throw UnknownKindException(kind)
        }
        if (r.pos != end) {
            throw CodecException("${end - r.pos} bytes after the message, which ends at byte ${r.pos - off}")
        }
        return msg
    }

    // ---- encoding ----

    private fun present(value: Any?): Int = if (value == null) 0 else 1

    private fun optUint(out: CborWriter, key: Int, value: Int?) {
        if (value != null) {
            out.key(key)
            out.uint(value)
        }
    }

    private fun optText(out: CborWriter, key: Int, value: String?) {
        if (value != null) {
            out.key(key)
            out.text(value)
        }
    }

    private fun uints(out: CborWriter, values: IntArray) {
        out.array(values.size)
        for (v in values) out.uint(v)
    }

    private fun encodeHello(m: Hello, out: CborWriter) {
        out.map(5 + present(m.token))
        out.key(0); out.uint(Kind.HELLO)
        out.key(1); uints(out, m.versions)
        out.key(2); out.uint(m.capabilities)
        out.key(3); out.uint(m.maxFrame)
        out.key(4); out.text(m.agent)
        optText(out, 5, m.token)
    }

    private fun encodeWelcome(m: Welcome, out: CborWriter) {
        out.map(3 + present(m.version) + present(m.error))
        out.key(0); out.uint(Kind.WELCOME)
        out.key(1); uints(out, m.versions)
        optUint(out, 2, m.version)
        out.key(3); out.uint(m.maxFrame)
        m.error?.let { out.key(4); encodeError(it, out) }
    }

    private fun encodeRequest(m: Request, out: CborWriter) {
        out.map(4 + present(m.deadlineMs))
        out.key(0); out.uint(Kind.REQUEST)
        out.key(1); out.uint(m.id)
        out.key(2); out.uint(m.call.method)
        optUint(out, 3, m.deadlineMs)
        out.key(4); encodeCall(m.call, out)
    }

    private fun encodeCall(c: Call, out: CborWriter) {
        when (c) {
            is SnapshotCall -> {
                out.map(
                    present(c.maxDepth) + present(c.maxNodes) + present(c.idleMs) + present(c.evictEventLess) +
                        present(c.evict),
                )
                optUint(out, 0, c.maxDepth)
                optUint(out, 1, c.maxNodes)
                optUint(out, 2, c.idleMs)
                c.evictEventLess?.let { out.key(3); out.bool(it) }
                c.evict?.let { out.key(4); uints(out, it) }
            }
            is ScreenshotCall -> {
                out.map(present(c.format) + present(c.quality) + present(c.maxDimension))
                optUint(out, 0, c.format)
                optUint(out, 1, c.quality)
                optUint(out, 2, c.maxDimension)
            }
            is TapCall -> {
                out.map(2 + present(c.durationMs))
                out.key(0); out.sint(c.x)
                out.key(1); out.sint(c.y)
                optUint(out, 2, c.durationMs)
            }
            is SwipeCall -> {
                out.map(5 + present(c.restMs))
                out.key(0); out.sint(c.x1)
                out.key(1); out.sint(c.y1)
                out.key(2); out.sint(c.x2)
                out.key(3); out.sint(c.y2)
                out.key(4); out.uint(c.durationMs)
                optUint(out, 5, c.restMs)
            }
            is KeyCall -> {
                out.map(1 + present(c.meta))
                out.key(0); out.uint(c.code)
                optUint(out, 1, c.meta)
            }
            is TextSetCall -> {
                out.map(1 + present(c.node) + present(c.mode))
                out.key(0); out.text(c.text)
                optUint(out, 1, c.node)
                optUint(out, 2, c.mode)
            }
            is FramesStartCall -> {
                out.map(present(c.maxFps) + present(c.maxDimension))
                optUint(out, 0, c.maxFps)
                optUint(out, 1, c.maxDimension)
            }
            is ResyncCall -> {
                out.map(1)
                out.key(0); out.uint(c.stream)
            }
            is BoundsCall -> {
                out.map(1)
                out.key(0); uints(out, c.nodes)
            }
            FramesStopCall, ShutdownCall, is UnknownCall -> out.map(0)
        }
    }

    private fun encodeResponse(m: Response, out: CborWriter) {
        out.map(4)
        out.key(0); out.uint(Kind.RESPONSE)
        out.key(1); out.uint(m.id)
        out.key(2); out.uint(m.method)
        val result = m.result
        val error = m.error
        if (result != null) {
            out.key(3); encodeReply(result, out)
        } else if (error != null) {
            out.key(4); encodeError(error, out)
        }
    }

    private fun encodeReply(r: Reply, out: CborWriter) {
        when (r) {
            is Snapshot -> encodeSnapshot(r, out)
            is Screenshot -> {
                out.map(5)
                out.key(0); out.uint(r.format)
                out.key(1); out.uint(r.width)
                out.key(2); out.uint(r.height)
                out.key(3); out.uint(r.rotation)
                out.key(4); out.bytes(r.data)
            }
            is InputAck -> {
                out.map(1)
                out.key(0); out.bool(r.accepted)
            }
            is TextAck -> {
                out.map(1)
                out.key(0); out.uint(r.method)
            }
            is ResyncAck -> {
                out.map(2)
                out.key(0); out.uint(r.stream)
                out.key(1); out.ulong(r.seq)
            }
            is Probed -> {
                out.map(1)
                out.key(0); out.array(r.nodes.size)
                for (n in r.nodes) {
                    out.array(2)
                    out.uint(n.node)
                    encodeBounds(n.bounds, out)
                }
            }
            EmptyReply -> out.map(0)
        }
    }

    private fun encodeSnapshot(s: Snapshot, out: CborWriter) {
        out.map(5)
        out.key(0); out.array(s.strings.size)
        for (str in s.strings) out.text(str)
        out.key(1); out.array(s.windows.size)
        for (w in s.windows) encodeWindow(w, out)
        out.key(2); out.uint(s.nodeCount)
        out.key(3); out.uint(s.truncated)
        out.key(4); out.ulong(s.seq)
    }

    private fun encodeWindow(w: Window, out: CborWriter) {
        val modal = if (w.modal) 1 else 0
        out.map(
            5 + present(w.packageIndex) + present(w.title) + present(w.root) + present(w.emitsEvents) +
                present(w.ownerIndex) + modal,
        )
        out.key(0); out.uint(w.id)
        out.key(1); out.uint(w.type)
        out.key(2); out.sint(w.layer)
        optUint(out, 3, w.packageIndex)
        optText(out, 4, w.title)
        out.key(5); encodeBounds(w.bounds, out)
        out.key(6); out.uint(w.flags)
        w.root?.let { out.key(7); encodeNode(it, out) }
        w.emitsEvents?.let { out.key(8); out.bool(it) }
        optUint(out, 9, w.ownerIndex)
        if (w.modal) {
            out.key(10); out.bool(true)
        }
    }

    private fun encodeNode(n: Node, out: CborWriter) {
        val children = if (n.children.isEmpty()) 0 else 1
        out.map(4 + present(n.resPkg) + present(n.resId) + present(n.text) + present(n.desc) + children + present(n.scroll))
        out.key(0); out.uint(n.id)
        out.key(1); out.uint(n.cls)
        optUint(out, 2, n.resPkg)
        optText(out, 3, n.resId)
        optText(out, 4, n.text)
        optText(out, 5, n.desc)
        out.key(6); encodeBounds(n.bounds, out)
        out.key(7); out.uint(n.flags)
        if (children != 0) {
            out.key(8); out.array(n.children.size)
            for (child in n.children) encodeNode(child, out)
        }
        n.scroll?.let { out.key(9); encodeScroll(it, out) }
    }

    private fun encodeScroll(s: Scroll, out: CborWriter) {
        out.map(2 + present(s.range) + present(s.position))
        out.key(0); out.uint(s.axis)
        out.key(1); out.uint(s.flags)
        s.range?.let {
            out.key(2); out.array(3)
            out.uint(it.unit)
            out.sint(it.min)
            out.sint(it.max)
        }
        s.position?.let {
            out.key(3); out.array(2)
            out.sint(it.first)
            out.sint(it.last)
        }
    }

    private fun encodeBounds(b: Bounds, out: CborWriter) {
        out.array(4)
        out.sint(b.left)
        out.sint(b.top)
        out.sint(b.right)
        out.sint(b.bottom)
    }

    private fun encodeError(e: WireError, out: CborWriter) {
        out.map(3)
        out.key(0); out.text(e.code)
        out.key(1); out.text(e.message)
        out.key(2); out.bool(e.retryable)
    }

    private fun encodeEvent(e: Event, out: CborWriter) {
        val p = e.payload
        val extra = when (p) {
            is TreeChange, is FrameAvailable -> 3
            is WindowChange, is Lifecycle -> 2
            is UnknownPayload -> 0
        }
        out.array(4 + extra)
        out.uint(Kind.EVENT)
        out.uint(p.stream)
        out.ulong(e.seq)
        out.ulong(e.timeUs)
        when (p) {
            is TreeChange -> {
                out.uint(p.change)
                out.uint(p.window)
                uints(out, p.path)
            }
            is WindowChange -> {
                out.uint(p.change)
                out.uint(p.window)
            }
            is FrameAvailable -> {
                out.uint(p.width)
                out.uint(p.height)
                out.uint(p.rotation)
            }
            is Lifecycle -> {
                out.uint(p.change)
                out.text(p.message)
            }
            is UnknownPayload -> {}
        }
    }

    private fun encodeNonce(kind: Int, nonce: Long, out: CborWriter) {
        out.map(2)
        out.key(0); out.uint(kind)
        out.key(1); out.ulong(nonce)
    }

    // ---- decoding ----

    private fun missing(field: String): Nothing = throw CodecException("the $field field is missing")

    private fun expectKind(found: Int, expected: Int) {
        if (found < 0) missing("kind")
        if (found != expected) throw CodecException("message kind $found, expected $expected")
    }

    /** The kind of the message at the reader's position, without moving it. */
    private fun peekKind(r: CborReader): Int {
        val p = CborReader(r.buf, r.pos, r.end)
        return when (p.peekMajor()) {
            4 -> {
                if (p.array() == 0) missing("kind")
                p.uint8()
            }
            5 -> {
                // Key 0 comes first in the encoders' layout; look further for others.
                repeat(p.map()) {
                    if (p.key() == 0L) return p.uint8()
                    p.skip()
                }
                missing("kind")
            }
            else -> throw CodecException("a message is a map or an array")
        }
    }

    private fun uints16(r: CborReader): IntArray = IntArray(r.array()) { r.uint16() }

    private fun uints32(r: CborReader): IntArray = IntArray(r.array()) { r.uint32() }

    private fun optError(r: CborReader): WireError? = if (r.nextIsNull()) { r.pos++; null } else decodeError(r)

    private fun decodeHello(r: CborReader): Hello {
        var kind = -1
        var versions: IntArray? = null
        var capabilities: Int? = null
        var maxFrame: Int? = null
        var agent: String? = null
        var token: String? = null
        repeat(r.map()) {
            when (r.key()) {
                0L -> kind = r.uint8()
                1L -> versions = uints16(r)
                2L -> capabilities = r.uint32()
                3L -> maxFrame = r.uint32()
                4L -> agent = r.text()
                5L -> token = r.optText()
                else -> r.skip()
            }
        }
        expectKind(kind, Kind.HELLO)
        return Hello(
            versions ?: missing("versions"),
            capabilities ?: missing("capabilities"),
            maxFrame ?: missing("max_frame"),
            agent ?: missing("agent"),
            token,
        )
    }

    private fun decodeWelcome(r: CborReader): Welcome {
        var kind = -1
        var versions: IntArray? = null
        var version: Int? = null
        var maxFrame: Int? = null
        var error: WireError? = null
        repeat(r.map()) {
            when (r.key()) {
                0L -> kind = r.uint8()
                1L -> versions = uints16(r)
                2L -> version = r.optUint16()
                3L -> maxFrame = r.uint32()
                4L -> error = optError(r)
                else -> r.skip()
            }
        }
        expectKind(kind, Kind.WELCOME)
        return Welcome(versions ?: missing("versions"), version, maxFrame ?: missing("max_frame"), error)
    }

    private fun decodeRequest(r: CborReader): Request {
        var kind = -1
        var id: Int? = null
        var method: Int? = null
        var deadlineMs: Int? = null
        var paramsAt = -1
        repeat(r.map()) {
            when (r.key()) {
                0L -> kind = r.uint8()
                1L -> id = r.uint32()
                2L -> method = r.uint16()
                3L -> deadlineMs = r.optUint32()
                // Decoded below, once the method is known whatever the key order.
                4L -> {
                    paramsAt = r.pos
                    r.skip()
                }
                else -> r.skip()
            }
        }
        expectKind(kind, Kind.REQUEST)
        val requestId = id ?: missing("id")
        val m = method ?: missing("method")
        if (paramsAt < 0) missing("params")
        return Request(requestId, deadlineMs, decodeCall(m, CborReader(r.buf, paramsAt, r.end)))
    }

    private fun decodeCall(method: Int, r: CborReader): Call = when (method) {
        Method.SNAPSHOT -> {
            var maxDepth: Int? = null
            var maxNodes: Int? = null
            var idleMs: Int? = null
            var evictEventLess: Boolean? = null
            var evict: IntArray? = null
            repeat(r.map()) {
                when (r.key()) {
                    0L -> maxDepth = r.optUint32()
                    1L -> maxNodes = r.optUint32()
                    2L -> idleMs = r.optUint32()
                    3L -> evictEventLess = r.optBool()
                    4L -> evict = if (r.nextIsNull()) { r.pos++; null } else uints32(r)
                    else -> r.skip()
                }
            }
            SnapshotCall(maxDepth, maxNodes, idleMs, evictEventLess, evict)
        }
        Method.SCREENSHOT -> {
            var format: Int? = null
            var quality: Int? = null
            var maxDimension: Int? = null
            repeat(r.map()) {
                when (r.key()) {
                    0L -> format = r.optUint8()
                    1L -> quality = r.optUint8()
                    2L -> maxDimension = r.optUint32()
                    else -> r.skip()
                }
            }
            ScreenshotCall(format, quality, maxDimension)
        }
        Method.TAP -> {
            var x: Int? = null
            var y: Int? = null
            var durationMs: Int? = null
            repeat(r.map()) {
                when (r.key()) {
                    0L -> x = r.int32()
                    1L -> y = r.int32()
                    2L -> durationMs = r.optUint32()
                    else -> r.skip()
                }
            }
            TapCall(x ?: missing("x"), y ?: missing("y"), durationMs)
        }
        Method.SWIPE -> {
            var x1: Int? = null
            var y1: Int? = null
            var x2: Int? = null
            var y2: Int? = null
            var durationMs: Int? = null
            var restMs: Int? = null
            repeat(r.map()) {
                when (r.key()) {
                    0L -> x1 = r.int32()
                    1L -> y1 = r.int32()
                    2L -> x2 = r.int32()
                    3L -> y2 = r.int32()
                    4L -> durationMs = r.uint32()
                    5L -> restMs = r.optUint32()
                    else -> r.skip()
                }
            }
            SwipeCall(
                x1 ?: missing("x1"),
                y1 ?: missing("y1"),
                x2 ?: missing("x2"),
                y2 ?: missing("y2"),
                durationMs ?: missing("duration_ms"),
                restMs,
            )
        }
        Method.KEY -> {
            var code: Int? = null
            var meta: Int? = null
            repeat(r.map()) {
                when (r.key()) {
                    0L -> code = r.uint32()
                    1L -> meta = r.optUint32()
                    else -> r.skip()
                }
            }
            KeyCall(code ?: missing("code"), meta)
        }
        Method.TEXT_SET -> {
            var text: String? = null
            var node: Int? = null
            var mode: Int? = null
            repeat(r.map()) {
                when (r.key()) {
                    0L -> text = r.text()
                    1L -> node = r.optUint32()
                    2L -> mode = r.optUint8()
                    else -> r.skip()
                }
            }
            TextSetCall(text ?: missing("text"), node, mode)
        }
        Method.FRAMES_START -> {
            var maxFps: Int? = null
            var maxDimension: Int? = null
            repeat(r.map()) {
                when (r.key()) {
                    0L -> maxFps = r.optUint32()
                    1L -> maxDimension = r.optUint32()
                    else -> r.skip()
                }
            }
            FramesStartCall(maxFps, maxDimension)
        }
        Method.FRAMES_STOP -> {
            r.skipMap()
            FramesStopCall
        }
        Method.RESYNC -> {
            var stream: Int? = null
            repeat(r.map()) {
                when (r.key()) {
                    0L -> stream = r.uint8()
                    else -> r.skip()
                }
            }
            ResyncCall(stream ?: missing("stream"))
        }
        Method.SHUTDOWN -> {
            r.skipMap()
            ShutdownCall
        }
        Method.BOUNDS -> {
            var nodes: IntArray? = null
            repeat(r.map()) {
                when (r.key()) {
                    0L -> nodes = uints32(r)
                    else -> r.skip()
                }
            }
            BoundsCall(nodes ?: missing("nodes"))
        }
        else -> {
            r.skip()
            UnknownCall(method)
        }
    }

    private fun decodeResponse(r: CborReader): Response {
        var kind = -1
        var id: Int? = null
        var method: Int? = null
        var resultAt = -1
        var error: WireError? = null
        repeat(r.map()) {
            when (r.key()) {
                0L -> kind = r.uint8()
                1L -> id = r.uint32()
                2L -> method = r.uint16()
                // Decoded below, once the method is known whatever the key order.
                3L -> {
                    resultAt = r.pos
                    r.skip()
                }
                4L -> error = optError(r)
                else -> r.skip()
            }
        }
        expectKind(kind, Kind.RESPONSE)
        val responseId = id ?: missing("id")
        val m = method ?: missing("method")
        val failure = error
        return when {
            resultAt >= 0 && failure == null ->
                Response(responseId, m, decodeReply(m, CborReader(r.buf, resultAt, r.end)), null)
            resultAt < 0 && failure != null -> Response(responseId, m, null, failure)
            resultAt >= 0 -> throw CodecException("a response with both a result and an error")
            else -> throw CodecException("a response with neither a result nor an error")
        }
    }

    private fun decodeReply(method: Int, r: CborReader): Reply = when (method) {
        Method.SNAPSHOT -> decodeSnapshot(r)
        Method.SCREENSHOT -> {
            var format: Int? = null
            var width: Int? = null
            var height: Int? = null
            var rotation: Int? = null
            var data: ByteArray? = null
            repeat(r.map()) {
                when (r.key()) {
                    0L -> format = r.uint8()
                    1L -> width = r.uint32()
                    2L -> height = r.uint32()
                    3L -> rotation = r.uint8()
                    4L -> data = r.bytes()
                    else -> r.skip()
                }
            }
            Screenshot(
                format ?: missing("format"),
                width ?: missing("width"),
                height ?: missing("height"),
                rotation ?: missing("rotation"),
                data ?: missing("data"),
            )
        }
        Method.TAP, Method.SWIPE, Method.KEY -> {
            var accepted: Boolean? = null
            repeat(r.map()) {
                when (r.key()) {
                    0L -> accepted = r.bool()
                    else -> r.skip()
                }
            }
            InputAck(accepted ?: missing("accepted"))
        }
        Method.TEXT_SET -> {
            var textMethod: Int? = null
            repeat(r.map()) {
                when (r.key()) {
                    0L -> textMethod = r.uint8()
                    else -> r.skip()
                }
            }
            TextAck(textMethod ?: missing("method"))
        }
        Method.RESYNC -> {
            var stream: Int? = null
            var seq: Long? = null
            repeat(r.map()) {
                when (r.key()) {
                    0L -> stream = r.uint8()
                    1L -> seq = r.uint64()
                    else -> r.skip()
                }
            }
            ResyncAck(stream ?: missing("stream"), seq ?: missing("seq"))
        }
        Method.BOUNDS -> {
            var nodes: Array<NodeBounds>? = null
            repeat(r.map()) {
                when (r.key()) {
                    0L -> nodes = Array(r.array()) { decodeNodeBounds(r) }
                    else -> r.skip()
                }
            }
            Probed(nodes ?: missing("nodes"))
        }
        Method.FRAMES_START, Method.FRAMES_STOP, Method.SHUTDOWN -> {
            r.skipMap()
            EmptyReply
        }
        else -> throw CodecException("a result for method $method, which this version does not know")
    }

    private fun decodeSnapshot(r: CborReader): Snapshot {
        var strings: Array<String>? = null
        var windows: Array<Window>? = null
        var nodeCount: Int? = null
        var truncated: Int? = null
        var seq: Long? = null
        repeat(r.map()) {
            when (r.key()) {
                0L -> strings = Array(r.array()) { r.text() }
                1L -> windows = Array(r.array()) { decodeWindow(r) }
                2L -> nodeCount = r.uint32()
                3L -> truncated = r.uint32()
                4L -> seq = r.uint64()
                else -> r.skip()
            }
        }
        return Snapshot(
            strings ?: missing("strings"),
            windows ?: missing("windows"),
            nodeCount ?: missing("node_count"),
            truncated ?: missing("truncated"),
            seq ?: missing("seq"),
        )
    }

    private fun decodeWindow(r: CborReader): Window {
        var id: Int? = null
        var type: Int? = null
        var layer: Int? = null
        var packageIndex: Int? = null
        var title: String? = null
        var bounds: Bounds? = null
        var flags: Int? = null
        var root: Node? = null
        var emitsEvents: Boolean? = null
        var ownerIndex: Int? = null
        var modal = false
        repeat(r.map()) {
            when (r.key()) {
                0L -> id = r.uint32()
                1L -> type = r.uint8()
                2L -> layer = r.int32()
                3L -> packageIndex = r.optUint32()
                4L -> title = r.optText()
                5L -> bounds = decodeBounds(r)
                6L -> flags = r.uint32()
                7L -> root = if (r.nextIsNull()) { r.pos++; null } else decodeNode(r, 0)
                8L -> emitsEvents = r.optBool()
                9L -> ownerIndex = r.optUint32()
                10L -> modal = r.bool()
                else -> r.skip()
            }
        }
        return Window(
            id ?: missing("id"),
            type ?: missing("kind"),
            layer ?: missing("layer"),
            packageIndex,
            title,
            bounds ?: missing("bounds"),
            flags ?: missing("flags"),
            root,
            emitsEvents,
            ownerIndex,
            modal,
        )
    }

    /** A node `depth` levels below its window root, which is at depth 0. */
    private fun decodeNode(r: CborReader, depth: Int): Node {
        var id: Int? = null
        var cls: Int? = null
        var resPkg: Int? = null
        var resId: String? = null
        var text: String? = null
        var desc: String? = null
        var bounds: Bounds? = null
        var flags: Int? = null
        var children = Node.NO_CHILDREN
        var scroll: Scroll? = null
        repeat(r.map()) {
            when (r.key()) {
                0L -> id = r.uint32()
                1L -> cls = r.uint32()
                2L -> resPkg = r.optUint32()
                3L -> resId = r.optText()
                4L -> text = r.optText()
                5L -> desc = r.optText()
                6L -> bounds = decodeBounds(r)
                7L -> flags = r.uint32()
                8L -> {
                    val n = r.array()
                    if (n > 0 && depth + 1 > Protocol.MAX_NODE_DEPTH) {
                        throw CodecException("node tree deeper than ${Protocol.MAX_NODE_DEPTH} levels")
                    }
                    children = Array(n) { decodeNode(r, depth + 1) }
                }
                9L -> scroll = if (r.nextIsNull()) { r.pos++; null } else decodeScroll(r)
                else -> r.skip()
            }
        }
        return Node(
            id ?: missing("id"),
            cls ?: missing("class"),
            resPkg,
            resId,
            text,
            desc,
            bounds ?: missing("bounds"),
            flags ?: missing("flags"),
            children,
            scroll,
        )
    }

    private fun decodeScroll(r: CborReader): Scroll {
        var axis: Int? = null
        var flags: Int? = null
        var range: ScrollRange? = null
        var position: ScrollPosition? = null
        repeat(r.map()) {
            when (r.key()) {
                0L -> axis = r.uint8()
                1L -> flags = r.uint32()
                2L -> range = if (r.nextIsNull()) { r.pos++; null } else decodeScrollRange(r)
                3L -> position = if (r.nextIsNull()) { r.pos++; null } else decodeScrollPosition(r)
                else -> r.skip()
            }
        }
        return Scroll(axis ?: missing("axis"), flags ?: missing("flags"), range, position)
    }

    private fun decodeScrollRange(r: CborReader): ScrollRange {
        val n = r.array()
        if (n < 3) throw CodecException("a scroll range of $n elements, expected 3")
        val range = ScrollRange(r.uint8(), r.int32(), r.int32())
        repeat(n - 3) { r.skip() }
        return range
    }

    private fun decodeScrollPosition(r: CborReader): ScrollPosition {
        val n = r.array()
        if (n < 2) throw CodecException("a scroll position of $n elements, expected 2")
        val position = ScrollPosition(r.int32(), r.int32())
        repeat(n - 2) { r.skip() }
        return position
    }

    /** A probed node, `[node, bounds]`, skipping elements past those this version knows. */
    private fun decodeNodeBounds(r: CborReader): NodeBounds {
        val n = r.array()
        if (n < 2) throw CodecException("a probed node of $n elements, expected 2")
        val probed = NodeBounds(r.uint32(), decodeBounds(r))
        repeat(n - 2) { r.skip() }
        return probed
    }

    private fun decodeBounds(r: CborReader): Bounds {
        val n = r.array()
        if (n < 4) throw CodecException("bounds of $n elements, expected 4")
        val bounds = Bounds(r.int32(), r.int32(), r.int32(), r.int32())
        repeat(n - 4) { r.skip() }
        return bounds
    }

    private fun decodeError(r: CborReader): WireError {
        var code: String? = null
        var message: String? = null
        var retryable: Boolean? = null
        repeat(r.map()) {
            when (r.key()) {
                0L -> code = r.text()
                1L -> message = r.text()
                2L -> retryable = r.bool()
                else -> r.skip()
            }
        }
        return WireError(code ?: missing("code"), message ?: missing("message"), retryable ?: missing("retryable"))
    }

    private fun decodeEvent(r: CborReader): Event {
        val n = r.array()
        if (n < 4) throw CodecException("an event of $n elements")
        expectKind(r.uint8(), Kind.EVENT)
        val stream = r.uint8()
        val seq = r.uint64()
        val timeUs = r.uint64()
        val known = when (stream) {
            Stream.TREE, Stream.FRAME -> 3
            Stream.WINDOW, Stream.LIFECYCLE -> 2
            else -> 0
        }
        if (n < 4 + known) throw CodecException("a stream $stream event of $n elements")
        val payload = when (stream) {
            Stream.TREE -> TreeChange(r.uint8(), r.uint32(), IntArray(r.array()) { r.uint32() })
            Stream.WINDOW -> WindowChange(r.uint8(), r.uint32())
            Stream.FRAME -> FrameAvailable(r.uint32(), r.uint32(), r.uint8())
            Stream.LIFECYCLE -> Lifecycle(r.uint8(), r.text())
            else -> UnknownPayload(stream)
        }
        repeat(n - 4 - known) { r.skip() }
        return Event(seq, timeUs, payload)
    }

    private fun decodeNonce(r: CborReader, expected: Int): Long {
        var kind = -1
        var nonce: Long? = null
        repeat(r.map()) {
            when (r.key()) {
                0L -> kind = r.uint8()
                1L -> nonce = r.uint64()
                else -> r.skip()
            }
        }
        expectKind(kind, expected)
        return nonce ?: missing("nonce")
    }
}
