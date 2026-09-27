package dev.mobileagent.agent

import dev.mobileagent.agent.codec.Capability
import dev.mobileagent.agent.codec.ErrorCode
import dev.mobileagent.agent.codec.Kind
import dev.mobileagent.agent.codec.Method
import dev.mobileagent.agent.codec.WireError

/**
 * Structured errors the agent sends, worded as crates/protocol words the
 * ones the host session produces itself, so a user sees one vocabulary.
 */
object Errors {
    fun of(code: String, message: String) = WireError(code, message, ErrorCode.retryable(code))

    fun methodName(method: Int): String = when (method) {
        Method.SNAPSHOT -> "snapshot"
        Method.SCREENSHOT -> "screenshot"
        Method.TAP -> "tap"
        Method.SWIPE -> "swipe"
        Method.KEY -> "key"
        Method.TEXT_SET -> "text_set"
        Method.FRAMES_START -> "frames_start"
        Method.FRAMES_STOP -> "frames_stop"
        Method.RESYNC -> "resync"
        Method.SHUTDOWN -> "shutdown"
        Method.BOUNDS -> "bounds"
        else -> method.toString()
    }

    fun kindName(kind: Int): String = when (kind) {
        Kind.HELLO -> "hello"
        Kind.WELCOME -> "welcome"
        Kind.REQUEST -> "request"
        Kind.RESPONSE -> "response"
        Kind.EVENT -> "event"
        Kind.PING -> "ping"
        Kind.PONG -> "pong"
        else -> "unknown kind"
    }

    /** The capability a method needs, 0 for none. */
    fun requiredCapability(method: Int): Int = when (method) {
        Method.SNAPSHOT -> Capability.TREE
        Method.SCREENSHOT -> Capability.SCREENSHOT
        Method.TAP, Method.SWIPE, Method.KEY -> Capability.INPUT
        Method.TEXT_SET -> Capability.TEXT_SET
        Method.FRAMES_START, Method.FRAMES_STOP -> Capability.FRAMES
        Method.RESYNC -> Capability.EVENTS
        Method.BOUNDS -> Capability.BOUNDS
        else -> 0
    }

    private val CAPABILITIES = arrayOf(
        Triple(Capability.TREE, "tree", "tree reads"),
        Triple(Capability.INPUT, "input", "input injection"),
        Triple(Capability.SCREENSHOT, "screenshot", "screenshots"),
        Triple(Capability.FRAMES, "frames", "frame streaming"),
        Triple(Capability.EVENTS, "events", "change events"),
        Triple(Capability.TEXT_SET, "text_set", "direct text setting"),
        Triple(Capability.BOUNDS, "bounds", "bounds probes"),
    )

    /** Capability names joined as the host prints a flag set: `tree | input`. */
    fun capabilityNames(bits: Int): String =
        CAPABILITIES.filter { bits and it.first != 0 }.joinToString(" | ") { it.second }

    private fun capabilityLabel(bits: Int): String =
        CAPABILITIES.filter { bits and it.first != 0 }.joinToString(" and ") { it.third }

    fun unknownMethod(method: Int) =
        of(ErrorCode.UNKNOWN_METHOD, "${methodName(method)} is not a method this agent implements")

    fun capabilityMissing(method: Int, missing: Int) = of(
        ErrorCode.CAPABILITY_MISSING,
        "the agent did not advertise ${capabilityLabel(missing)} (${capabilityNames(missing)}), which ${methodName(method)} needs",
    )

    fun deadlineExceeded(method: Int, id: Int, deadlineMs: Int) = of(
        ErrorCode.DEADLINE_EXCEEDED,
        "the ${methodName(method)} request ${id.toUInt()} did not finish within its ${deadlineMs.toUInt()} ms deadline",
    )

    fun frameTooLarge(method: Int, id: Int, size: Long, limit: Long) = of(
        ErrorCode.FRAME_TOO_LARGE,
        "the ${methodName(method)} message for request ${id.toUInt()} is $size bytes, over the negotiated maximum frame size of $limit bytes",
    )

    fun invalidParams(message: String) = of(ErrorCode.INVALID_PARAMS, message)

    fun internal(method: Int, t: Throwable) =
        of(ErrorCode.INTERNAL, "${methodName(method)} failed inside the agent: ${t.javaClass.name}: ${t.message}")
}
