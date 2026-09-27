package dev.mobileagent.agent.codec

// The protocol's messages, one for one with crates/protocol. Unsigned 32-bit
// fields are Ints and 64-bit ones Longs holding the bit pattern, so -1 stands
// for u32::MAX or u64::MAX. Optional fields are null when absent.

/** One message, the payload of one frame. */
sealed class Message(@JvmField val kind: Int)

/** The agent's first message on every connection. */
class Hello(
    @JvmField val versions: IntArray,
    @JvmField val capabilities: Int,
    @JvmField val maxFrame: Int,
    @JvmField val agent: String,
    @JvmField val token: String?,
) : Message(Kind.HELLO)

/** The host's answer to a hello: the selected version, or the error it closes with. */
class Welcome(
    @JvmField val versions: IntArray,
    @JvmField val version: Int?,
    @JvmField val maxFrame: Int,
    @JvmField val error: WireError?,
) : Message(Kind.WELCOME)

/** A request from the host; `deadlineMs` is relative to its arrival. */
class Request(
    @JvmField val id: Int,
    @JvmField val deadlineMs: Int?,
    @JvmField val call: Call,
) : Message(Kind.REQUEST)

/** The answer to request `id`: exactly one of `result` and `error`. */
class Response(
    @JvmField val id: Int,
    @JvmField val method: Int,
    @JvmField val result: Reply?,
    @JvmField val error: WireError?,
) : Message(Kind.RESPONSE) {
    init {
        require((result == null) != (error == null)) { "a response carries a result or an error" }
    }
}

/** An event; `seq` counts per stream from 1. */
class Event(
    @JvmField val seq: Long,
    @JvmField val timeUs: Long,
    @JvmField val payload: EventPayload,
) : Message(Kind.EVENT)

class Ping(@JvmField val nonce: Long) : Message(Kind.PING)

class Pong(@JvmField val nonce: Long) : Message(Kind.PONG)

/** A structured error: a snake_case code from [ErrorCode], a message, and whether retrying helps. */
class WireError(
    @JvmField val code: String,
    @JvmField val message: String,
    @JvmField val retryable: Boolean,
)

/** A method with its parameters. */
sealed class Call(@JvmField val method: Int)

class SnapshotCall(
    @JvmField val maxDepth: Int? = null,
    @JvmField val maxNodes: Int? = null,
    @JvmField val idleMs: Int? = null,
    /** Evict the cache of the windows that send no change events first: input the agent did not inject left them stale. */
    @JvmField val evictEventLess: Boolean? = null,
    /** Windows, by id, whose cached nodes to evict first: bounds probes found their nodes moved without a change event. */
    @JvmField val evict: IntArray? = null,
) : Call(Method.SNAPSHOT)

class ScreenshotCall(
    @JvmField val format: Int? = null,
    @JvmField val quality: Int? = null,
    @JvmField val maxDimension: Int? = null,
) : Call(Method.SCREENSHOT)

/** A tap, or a long press when `durationMs` is set. */
class TapCall(
    @JvmField val x: Int,
    @JvmField val y: Int,
    @JvmField val durationMs: Int? = null,
) : Call(Method.TAP)

class SwipeCall(
    @JvmField val x1: Int,
    @JvmField val y1: Int,
    @JvmField val x2: Int,
    @JvmField val y2: Int,
    @JvmField val durationMs: Int,
) : Call(Method.SWIPE)

class KeyCall(@JvmField val code: Int, @JvmField val meta: Int? = null) : Call(Method.KEY)

/** Text for `node`, or for the focused field when `node` is null. */
class TextSetCall(
    @JvmField val text: String,
    @JvmField val node: Int? = null,
    @JvmField val mode: Int? = null,
) : Call(Method.TEXT_SET)

class FramesStartCall(
    @JvmField val maxFps: Int? = null,
    @JvmField val maxDimension: Int? = null,
) : Call(Method.FRAMES_START)

object FramesStopCall : Call(Method.FRAMES_STOP)

class ResyncCall(@JvmField val stream: Int) : Call(Method.RESYNC)

object ShutdownCall : Call(Method.SHUTDOWN)

/** Nodes of the last snapshot, by id, whose bounds to read again past the platform's node cache. */
class BoundsCall(@JvmField val nodes: IntArray) : Call(Method.BOUNDS)

/** A method this version does not know; its parameters were skipped. */
class UnknownCall(method: Int) : Call(method)

/** The result of a successful request; which one follows from the method. */
sealed class Reply

/** A tree read: nodes refer to `strings` for class names and prefixes. */
class Snapshot(
    @JvmField val strings: Array<String>,
    @JvmField val windows: Array<Window>,
    @JvmField val nodeCount: Int,
    @JvmField val truncated: Int,
    @JvmField val seq: Long,
) : Reply()

class Window(
    @JvmField val id: Int,
    @JvmField val type: Int,
    @JvmField val layer: Int,
    @JvmField val packageIndex: Int?,
    @JvmField val title: String?,
    @JvmField val bounds: Bounds,
    @JvmField val flags: Int,
    @JvmField val root: Node?,
    /** Whether the window's toolkit sends change events; null when the agent cannot tell. */
    @JvmField val emitsEvents: Boolean? = null,
    /** String-table index of the system process that owns the window; null for the application under automation. */
    @JvmField val ownerIndex: Int? = null,
    /** The window keeps the user from the windows below it until it is dismissed. */
    @JvmField val modal: Boolean = false,
)

class Node(
    @JvmField val id: Int,
    @JvmField val cls: Int,
    @JvmField val resPkg: Int?,
    @JvmField val resId: String?,
    @JvmField val text: String?,
    @JvmField val desc: String?,
    @JvmField val bounds: Bounds,
    @JvmField val flags: Int,
    @JvmField val children: Array<Node>,
    /** How the node scrolls; null on nodes that do not scroll. */
    @JvmField val scroll: Scroll? = null,
) {
    companion object {
        @JvmField val NO_CHILDREN: Array<Node> = arrayOf()
    }
}

/**
 * Scroll semantics: the [ScrollAxis], [ScrollFlag] bits for the ways the node
 * can scroll from where it is, and, when known, the range of its content and
 * the part of that range inside the viewport.
 */
class Scroll(
    @JvmField val axis: Int,
    @JvmField val flags: Int,
    @JvmField val range: ScrollRange? = null,
    /** Only with a range. */
    @JvmField val position: ScrollPosition? = null,
)

/** The span of a scrolling node's content in [ScrollUnit] `unit`, both ends inclusive. */
class ScrollRange(@JvmField val unit: Int, @JvmField val min: Int, @JvmField val max: Int)

/** The part of a [ScrollRange] inside the viewport, both ends inclusive. */
class ScrollPosition(@JvmField val first: Int, @JvmField val last: Int)

class Bounds(
    @JvmField val left: Int,
    @JvmField val top: Int,
    @JvmField val right: Int,
    @JvmField val bottom: Int,
)

class Screenshot(
    @JvmField val format: Int,
    @JvmField val width: Int,
    @JvmField val height: Int,
    @JvmField val rotation: Int,
    @JvmField val data: ByteArray,
) : Reply()

class InputAck(@JvmField val accepted: Boolean) : Reply()

class TextAck(@JvmField val method: Int) : Reply()

class ResyncAck(@JvmField val stream: Int, @JvmField val seq: Long) : Reply()

/** The result of bounds: the nodes still on screen with their bounds now, in the request's order. */
class Probed(@JvmField val nodes: Array<NodeBounds>) : Reply()

/** One node of a [Probed]: its id in the last snapshot and its bounds. */
class NodeBounds(@JvmField val node: Int, @JvmField val bounds: Bounds)

/** The empty result of frames_start, frames_stop and shutdown. */
object EmptyReply : Reply()

/** What an event says; the class determines the stream. */
sealed class EventPayload(@JvmField val stream: Int)

class TreeChange(
    @JvmField val change: Int,
    @JvmField val window: Int,
    @JvmField val path: IntArray,
) : EventPayload(Stream.TREE)

class WindowChange(@JvmField val change: Int, @JvmField val window: Int) : EventPayload(Stream.WINDOW)

class FrameAvailable(
    @JvmField val width: Int,
    @JvmField val height: Int,
    @JvmField val rotation: Int,
) : EventPayload(Stream.FRAME)

class Lifecycle(@JvmField val change: Int, @JvmField val message: String) : EventPayload(Stream.LIFECYCLE)

/** An event on a stream this version does not know; its elements were skipped. */
class UnknownPayload(stream: Int) : EventPayload(stream)
