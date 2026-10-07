package dev.mobileagent.agent.codec

// Constants of the device protocol, one for one with crates/protocol. Its
// README describes the wire layout this codec writes.

/** Versions and limits. */
object Protocol {
    /** The protocol version this codec speaks. */
    const val VERSION = 1

    /** Every protocol version this codec speaks, lowest first. */
    @JvmField val VERSIONS: IntArray = intArrayOf(VERSION)

    /** Default maximum frame payload: 16 MiB. */
    const val DEFAULT_MAX_FRAME = 16 * 1024 * 1024

    /** Smallest maximum frame payload a side may negotiate: 4 KiB. */
    const val MIN_MAX_FRAME = 4 * 1024

    /** Deepest node nesting below a window root that decoders accept. */
    const val MAX_NODE_DEPTH = 256
}

/** Message kinds: key 0 of a map-shaped message, element 0 of an event. */
object Kind {
    const val HELLO = 0
    const val WELCOME = 1
    const val REQUEST = 2
    const val RESPONSE = 3
    const val EVENT = 4
    const val PING = 5
    const val PONG = 6
}

/** Request methods. */
object Method {
    const val SNAPSHOT = 1
    const val SCREENSHOT = 2
    const val TAP = 3
    const val SWIPE = 4
    const val KEY = 5
    const val TEXT_SET = 6
    const val FRAMES_START = 7
    const val FRAMES_STOP = 8
    const val RESYNC = 9
    const val SHUTDOWN = 10
    const val BOUNDS = 11
}

/** Capability bits of a hello. */
object Capability {
    const val TREE = 1 shl 0
    const val INPUT = 1 shl 1
    const val SCREENSHOT = 1 shl 2
    const val FRAMES = 1 shl 3
    const val EVENTS = 1 shl 4
    const val TEXT_SET = 1 shl 5
    const val BOUNDS = 1 shl 6
}

/** Event streams. Frame availability is lossy; the others are lossless. */
object Stream {
    const val TREE = 0
    const val WINDOW = 1
    const val FRAME = 2
    const val LIFECYCLE = 3
}

/** Error codes, each with the retryable flag the host expects for it. */
object ErrorCode {
    const val UNKNOWN_METHOD = "unknown_method"
    const val INVALID_PARAMS = "invalid_params"
    const val CAPABILITY_MISSING = "capability_missing"
    const val DEADLINE_EXCEEDED = "deadline_exceeded"
    const val BUSY = "busy"
    const val WINDOW_CHANGED = "window_changed"
    const val FRAME_TOO_LARGE = "frame_too_large"
    const val VERSION_MISMATCH = "version_mismatch"
    const val TOKEN_MISMATCH = "token_mismatch"
    const val FRAME_LIMIT_TOO_SMALL = "frame_limit_too_small"
    const val NOT_READY = "not_ready"
    const val DISCONNECTED = "disconnected"
    const val PROTOCOL_ERROR = "protocol_error"
    const val INTERNAL = "internal"

    /** Whether retrying is meaningful for a code this version defines. */
    fun retryable(code: String): Boolean =
        code == DEADLINE_EXCEEDED || code == BUSY || code == WINDOW_CHANGED || code == NOT_READY
}

/** Node state bits. */
object NodeFlag {
    const val CLICKABLE = 1 shl 0
    const val LONG_CLICKABLE = 1 shl 1
    const val FOCUSABLE = 1 shl 2
    const val FOCUSED = 1 shl 3
    const val ENABLED = 1 shl 4
    const val CHECKABLE = 1 shl 5
    const val CHECKED = 1 shl 6
    const val SCROLLABLE = 1 shl 7
    const val SELECTED = 1 shl 8
    const val EDITABLE = 1 shl 9
    const val PASSWORD = 1 shl 10
    const val VISIBLE = 1 shl 11
    const val ACCESSIBILITY_FOCUSED = 1 shl 12

    // Bit 13, modal, is set by the Simulator's reader alone.

    /** Drawn after every sibling its bounds overlap, by the order the platform draws them in. */
    const val ON_TOP = 1 shl 14
}

/** Window state bits. */
object WindowFlag {
    const val ACTIVE = 1 shl 0
    const val FOCUSED = 1 shl 1
    const val ACCESSIBILITY_FOCUSED = 1 shl 2
    const val PICTURE_IN_PICTURE = 1 shl 3

    /** Drawn away from where its layout puts it, as while the platform animates the window in or out. */
    const val TRANSFORMED = 1 shl 4
}

/** The direction a node scrolls in. */
object ScrollAxis {
    /** The platform names no single direction. */
    const val UNKNOWN = 0
    const val VERTICAL = 1
    const val HORIZONTAL = 2
}

/** Which ways a node can scroll from where it is. */
object ScrollFlag {
    /** Toward the start: up or left. */
    const val BACK = 1 shl 0

    /** Toward the end: down or right. */
    const val FORWARD = 1 shl 1
}

/** What a scroll range counts. */
object ScrollUnit {
    /** A collection's items along the axis, numbered from 0. */
    const val ITEMS = 0

    /** Percent of the scrollable range, 0 to 100. */
    const val PERCENT = 1
}

/** Which request bounds cut a snapshot short. */
object Truncation {
    const val DEPTH = 1 shl 0
    const val NODES = 1 shl 1
    const val WINDOWS = 1 shl 2
}

/** Window kinds, with the values of `AccessibilityWindowInfo.TYPE_*`. */
object WindowType {
    const val UNKNOWN = 0
    const val APPLICATION = 1
    const val INPUT_METHOD = 2
    const val SYSTEM = 3
    const val ACCESSIBILITY_OVERLAY = 4
    const val SPLIT_SCREEN_DIVIDER = 5
    const val MAGNIFICATION_OVERLAY = 6
}

/** What changed in a tree. */
object TreeChangeKind {
    const val CONTENT = 0
    const val TEXT = 1
    const val SCROLL = 2
    const val FOCUS = 3
    const val SELECTION = 4
}

/** What changed about a window. */
object WindowChangeKind {
    const val STATE = 0
    const val ADDED = 1
    const val REMOVED = 2
    const val FOCUS = 3
    const val BOUNDS = 4
}

/** What happened to the agent. */
object LifecycleKind {
    const val READY = 0
    const val OWNERSHIP_LOST = 1
    const val SHUTTING_DOWN = 2
    const val WARNING = 3
}

/** Image encodings. */
object ImageFormat {
    const val PNG = 0
    const val JPEG = 1
}

/** Display rotation, with the values of `Surface.ROTATION_*`. */
object Rotation {
    const val R0 = 0
    const val R90 = 1
    const val R180 = 2
    const val R270 = 3
}

/** How text is entered; AUTO only in requests. */
object TextMethod {
    const val AUTO = 0
    const val DIRECT = 1
    const val KEYS = 2
}
