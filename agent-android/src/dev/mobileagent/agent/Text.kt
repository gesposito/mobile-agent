package dev.mobileagent.agent

import android.os.Bundle
import android.view.InputDevice
import android.view.KeyCharacterMap
import android.view.KeyEvent
import android.view.accessibility.AccessibilityNodeInfo
import dev.mobileagent.agent.codec.TextMethod

/** A text request that cannot be carried out as asked; answered as invalid parameters. */
class TextRefused(message: String) : Exception(message)

/**
 * Text for an editable field: set directly with `ACTION_SET_TEXT` when the
 * node offers it, typed as key events otherwise (spec android-driver, Direct
 * text setting with fallback). Both replace what the field held.
 */
class TextSetter(private val automation: Automation, private val input: Input?) {
    private val keyMap: KeyCharacterMap by lazy { KeyCharacterMap.load(KeyCharacterMap.VIRTUAL_KEYBOARD) }

    /**
     * Sets `text` in `node`, or in the node with input focus when null, as
     * `mode` asks, and returns the method used.
     */
    fun set(text: String, node: AccessibilityNodeInfo?, mode: Int): Int {
        val target = node ?: automation.uia.findFocus(AccessibilityNodeInfo.FOCUS_INPUT)
            ?: throw TextRefused("no element has input focus; tap a field first or name a node from the last snapshot")
        // Its actions and text as they are now, not as a cached read had them.
        target.refresh()
        val direct = target.isEditable &&
            target.actionList.any { it.id == AccessibilityNodeInfo.AccessibilityAction.ACTION_SET_TEXT.id }
        return when (mode) {
            TextMethod.DIRECT -> {
                if (!direct) throw TextRefused("the element does not accept direct text setting; ask for keys or let the agent choose")
                if (!setDirectly(target, text)) throw TextRefused("the element refused the text")
                TextMethod.DIRECT
            }
            TextMethod.KEYS -> {
                type(target, text)
                TextMethod.KEYS
            }
            else -> if (direct && setDirectly(target, text)) {
                TextMethod.DIRECT
            } else {
                type(target, text)
                TextMethod.KEYS
            }
        }
    }

    private fun setDirectly(target: AccessibilityNodeInfo, text: String): Boolean {
        val args = Bundle()
        args.putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, text)
        return target.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, args)
    }

    /**
     * Types `text` as the virtual keyboard would, after giving `target` input
     * focus and selecting what an editable field held, so the keys replace it.
     */
    private fun type(target: AccessibilityNodeInfo, text: String) {
        val keys = input ?: throw TextRefused("this device offers no input injection, so text can only be set directly")
        val events = keyMap.getEvents(text.toCharArray()) ?: run {
            val bad = text.first { keyMap.getEvents(charArrayOf(it)) == null }
            throw TextRefused(
                "U+%04X ('%c') has no key on the virtual keyboard, so it cannot be typed; set the text directly".format(bad.code, bad),
            )
        }
        if (!target.isFocused) target.performAction(AccessibilityNodeInfo.ACTION_FOCUS)
        if (target.isEditable && !target.text.isNullOrEmpty() && !target.isShowingHintText) {
            keys.key(KeyEvent.KEYCODE_A, KeyEvent.META_CTRL_ON or KeyEvent.META_CTRL_LEFT_ON)
        }
        for (e in events) e.source = InputDevice.SOURCE_KEYBOARD
        if (!keys.keys(events)) throw TextRefused("the platform refused the key events")
    }
}
