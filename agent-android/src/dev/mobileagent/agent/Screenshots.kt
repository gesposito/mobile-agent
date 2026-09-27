package dev.mobileagent.agent

import android.graphics.Bitmap
import dev.mobileagent.agent.codec.ImageFormat
import dev.mobileagent.agent.codec.Rotation
import dev.mobileagent.agent.codec.Screenshot
import java.io.ByteArrayOutputStream

/**
 * Screenshots through the public `UiAutomation.takeScreenshot()` only (design
 * D2): the hidden capture plumbing changed in Android 16, 16 QPR2 and 17. The
 * platform returns the display as the user sees it, so a landscape display
 * gives a landscape, upright image (spike S1).
 */
class Screenshots(private val automation: Automation) {
    companion object {
        const val DEFAULT_QUALITY = 80
    }

    /** Encodes the display; `maxDimension` bounds the longer side. */
    fun capture(format: Int, quality: Int, maxDimension: Int?): Screenshot {
        val rotation = displayRotation()
        val full = automation.uia.takeScreenshot() ?: throw IllegalStateException("the platform returned no screenshot")
        var image = full
        val longer = maxOf(full.width, full.height)
        if (maxDimension != null && maxDimension in 1 until longer) {
            val w = if (full.width >= full.height) maxDimension else (full.width.toLong() * maxDimension / full.height).toInt()
            val h = if (full.width >= full.height) (full.height.toLong() * maxDimension / full.width).toInt() else maxDimension
            image = Bitmap.createScaledBitmap(full, maxOf(1, w), maxOf(1, h), true)
            full.recycle()
        }
        val out = ByteArrayOutputStream(image.width * image.height / 4)
        val compress = if (format == ImageFormat.PNG) Bitmap.CompressFormat.PNG else Bitmap.CompressFormat.JPEG
        image.compress(compress, quality, out)
        val shot = Screenshot(
            format,
            image.width,
            image.height,
            if (rotation >= 0) rotation else if (image.width > image.height) Rotation.R90 else Rotation.R0,
            out.toByteArray(),
        )
        image.recycle()
        return shot
    }

    /**
     * The default display's rotation, `Surface.ROTATION_*`, from the hidden
     * `DisplayManagerGlobal.getDisplayInfo`; -1 when it cannot be read.
     */
    private fun displayRotation(): Int = try {
        val cls = Class.forName("android.hardware.display.DisplayManagerGlobal")
        val global = cls.getDeclaredMethod("getInstance").invoke(null)
        val info = cls.getDeclaredMethod("getDisplayInfo", Int::class.javaPrimitiveType).invoke(global, 0)
        info!!.javaClass.getField("rotation").getInt(info)
    } catch (e: Exception) {
        -1
    }
}
