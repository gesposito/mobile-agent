package dev.mobileagent.agent

import android.os.SystemClock
import android.view.InputDevice
import android.view.InputEvent
import android.view.KeyCharacterMap
import android.view.KeyEvent
import android.view.MotionEvent
import java.lang.reflect.InvocationTargetException
import java.lang.reflect.Method

/**
 * Input injection through the hidden
 * `InputManagerGlobal.injectInputEvent(event, INJECT_INPUT_EVENT_MODE_WAIT_FOR_RESULT)`,
 * which returns once dispatch has decided whether the event is permitted; its
 * result is the accepted flag the acknowledgement carries. Spike S1 measured a
 * tap (DOWN and UP) at 1.9 ms p50 and 3.1 ms p95. The UiAutomation overloads
 * are never used: with sync they wait for the app (35 to 45 ms p50), without
 * it they inject asynchronously and always report success. Validated on API
 * 36; `InputManagerGlobal` exists from API 34, `InputManager` before it.
 */
class Input {
    companion object {
        /** `InputManager.INJECT_INPUT_EVENT_MODE_WAIT_FOR_RESULT`. */
        private const val WAIT_FOR_RESULT = 1

        /** Time between the moves of a gesture, about one per display frame at 120 Hz. */
        const val STEP_MS = 8L
    }

    private val manager: Any
    private val inject: Method

    init {
        val cls = try {
            Class.forName("android.hardware.input.InputManagerGlobal")
        } catch (e: ClassNotFoundException) {
            Class.forName("android.hardware.input.InputManager")
        }
        manager = cls.getDeclaredMethod("getInstance").invoke(null)!!
        inject = cls.getDeclaredMethod("injectInputEvent", InputEvent::class.java, Int::class.javaPrimitiveType)
    }

    /** Injects one event and returns whether the platform accepted it. */
    fun inject(event: InputEvent): Boolean = try {
        inject.invoke(manager, event, WAIT_FOR_RESULT) as Boolean
    } catch (e: InvocationTargetException) {
        // A refusal the platform throws, such as a SecurityException, is a refusal.
        false
    }

    private fun motion(action: Int, downTime: Long, x: Float, y: Float): MotionEvent {
        val props = arrayOf(
            MotionEvent.PointerProperties().apply {
                id = 0
                toolType = MotionEvent.TOOL_TYPE_FINGER
            },
        )
        val coords = arrayOf(
            MotionEvent.PointerCoords().apply {
                this.x = x
                this.y = y
                pressure = 1f
                size = 1f
            },
        )
        return MotionEvent.obtain(
            downTime, SystemClock.uptimeMillis(), action, 1, props, coords,
            0, 0, 1f, 1f, 0, 0, InputDevice.SOURCE_TOUCHSCREEN, 0,
        )
    }

    /**
     * One pointer down at the first point, moved through the others at their
     * times (ms after the first), and up at the last. The pointer follows the
     * real clock, so the app sees the velocity the times describe. When
     * `cancelled` turns true mid-gesture the pointer is cancelled, never left
     * down. Returns whether every event was accepted.
     */
    fun gesture(xs: FloatArray, ys: FloatArray, times: LongArray, cancelled: () -> Boolean): Boolean {
        val down = SystemClock.uptimeMillis()
        var accepted = inject(motion(MotionEvent.ACTION_DOWN, down, xs[0], ys[0]))
        val last = xs.size - 1
        for (i in 1..last) {
            val wait = down + times[i] - SystemClock.uptimeMillis()
            if (wait > 0) Thread.sleep(wait)
            if (cancelled()) {
                inject(motion(MotionEvent.ACTION_CANCEL, down, xs[i], ys[i]))
                throw Cancelled()
            }
            val action = if (i == last) MotionEvent.ACTION_UP else MotionEvent.ACTION_MOVE
            accepted = inject(motion(action, down, xs[i], ys[i])) && accepted
        }
        return accepted
    }

    /** A tap, or a press held for `holdMs`. */
    fun tap(x: Int, y: Int, holdMs: Long, cancelled: () -> Boolean): Boolean =
        gesture(floatArrayOf(x.toFloat(), x.toFloat()), floatArrayOf(y.toFloat(), y.toFloat()), longArrayOf(0, holdMs), cancelled)

    /**
     * A straight swipe lasting `durationMs`: down at the start, a move every
     * [STEP_MS] along the line, the last one at the end, then up there.
     */
    fun swipe(x1: Int, y1: Int, x2: Int, y2: Int, durationMs: Long, cancelled: () -> Boolean): Boolean {
        val steps = maxOf(1, (durationMs / STEP_MS).toInt())
        val xs = FloatArray(steps + 2)
        val ys = FloatArray(steps + 2)
        val ts = LongArray(steps + 2)
        for (k in 0..steps) {
            xs[k] = x1 + (x2 - x1) * k.toFloat() / steps
            ys[k] = y1 + (y2 - y1) * k.toFloat() / steps
            ts[k] = durationMs * k / steps
        }
        xs[steps + 1] = x2.toFloat()
        ys[steps + 1] = y2.toFloat()
        ts[steps + 1] = durationMs
        return gesture(xs, ys, ts, cancelled)
    }

    /** A key pressed and released with the given meta state, as the virtual keyboard sends it. */
    fun key(code: Int, meta: Int): Boolean {
        val down = SystemClock.uptimeMillis()
        val press = KeyEvent(down, down, KeyEvent.ACTION_DOWN, code, 0, meta, KeyCharacterMap.VIRTUAL_KEYBOARD, 0, 0, InputDevice.SOURCE_KEYBOARD)
        val accepted = inject(press)
        val release = KeyEvent(down, SystemClock.uptimeMillis(), KeyEvent.ACTION_UP, code, 0, meta, KeyCharacterMap.VIRTUAL_KEYBOARD, 0, 0, InputDevice.SOURCE_KEYBOARD)
        return inject(release) && accepted
    }

    /** Injects key events as they are, returning whether all were accepted. */
    fun keys(events: Array<KeyEvent>): Boolean {
        var accepted = true
        for (e in events) accepted = inject(e) && accepted
        return accepted
    }
}
