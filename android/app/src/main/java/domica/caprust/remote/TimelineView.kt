package domica.caprust.remote

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.RectF
import android.view.MotionEvent
import android.view.View
import org.json.JSONArray
import org.json.JSONObject
import kotlin.math.max

/**
 * A drawn map of the timeline: one row per track, clips as blocks
 * placed by start time and sized by duration.
 *
 * Drawn rather than composed from views. Forty clips as forty
 * Buttons is forty layouts and a scroll list that reads as a list; a
 * Canvas gives proportional blocks, which is the thing that makes the
 * shape of an edit visible at a glance.
 *
 * Tapping selects. Dragging is deliberately not here yet: a finger is
 * roughly ten pixels, so a drag that moves a clip by a frame is a
 * drag that is unusable without snap and a zoomable time scale, and
 * those are a separate piece of work rather than an extra gesture
 * handler.
 */
class TimelineView(context: Context) : View(context) {

    /** Colour per track, cycled. Distinct enough to tell rows apart. */
    private val palette = intArrayOf(
        0xFF4A90D9.toInt(), 0xFF7BC96F.toInt(), 0xFFE0A458.toInt(),
        0xFFB07CD6.toInt(), 0xFFDB6B8A.toInt(), 0xFF5FC9C9.toInt(),
    )

    private val blockPaint = Paint(Paint.ANTI_ALIAS_FLAG)
    private val textPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = 0xFFFFFFFF.toInt()
        textSize = 11f * resources.displayMetrics.scaledDensity
    }
    private val trackPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = 0x22FFFFFF }
    private val gridPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = 0x33FFFFFF.toInt() }
    private val emptyPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = 0x88FFFFFF.toInt() }

    private val rowHeight = 34f * resources.displayMetrics.density
    private val gap = 3f * resources.displayMetrics.density
    private val labelWidth = 34f * resources.displayMetrics.density

    private var clips: JSONArray = JSONArray()
    private var totalMs: Long = 1

    /** Blocks laid out during draw, so a tap can be hit-tested against
     *  exactly what was drawn rather than a second computation that
     *  could drift from it. */
    private val blocks = ArrayList<RectF>()
    private val blockIds = ArrayList<String>()

    /** Called when a block is tapped. */
    var onPick: ((String) -> Unit)? = null

    fun setClips(list: JSONArray, durationMs: Long) {
        clips = list
        totalMs = max(1L, durationMs)
        invalidate()
    }

    private fun trackCount(): Int {
        var maxTrack = 0
        for (i in 0 until clips.length()) {
            maxTrack = max(maxTrack, clips.getJSONObject(i).optInt("track_index", 0))
        }
        return maxTrack + 1
    }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        blocks.clear()
        blockIds.clear()

        val rows = trackCount()
        val usable = width - labelWidth
        // Seconds per pixel, so a 30s project and a 10-minute one both
        // fit the screen. Reading the shape never requires zooming.
        val msPerPx = totalMs.toFloat() / usable.coerceAtLeast(1)

        // Horizontal grid every second, labelled every ten.
        val stepMs = if (msPerPx * 1000f < 40f) 10_000L else 1_000L
        var t = 0L
        while (t <= totalMs) {
            val x = labelWidth + t / msPerPx
            canvas.drawLine(x, 0f, x, rows * (rowHeight + gap), gridPaint)
            t += stepMs
        }

        for (row in 0 until rows) {
            val top = row * (rowHeight + gap)
            canvas.drawRect(labelWidth, top, width.toFloat(), top + rowHeight, trackPaint)
            canvas.drawText("V$row", 4f, top + rowHeight * 0.7f, textPaint)

            for (i in 0 until clips.length()) {
                val c = clips.getJSONObject(i)
                if (c.optInt("track_index", 0) != row) continue
                val start = c.optLong("start_time_ms")
                val dur = c.optLong("duration_ms", 1)
                val left = labelWidth + start / msPerPx
                val right = labelWidth + (start + dur) / msPerPx
                // Never narrower than a few pixels: a one-frame clip at
                // a ten-minute zoom would vanish, and "I cannot see it"
                // reads as "it is not there".
                val w = max(right - left, 4f * resources.displayMetrics.density)
                val r = RectF(left, top + 2f, left + w, top + rowHeight - 2f)
                blocks.add(r)
                blockIds.add(c.optString("id"))

                blockPaint.color = palette[row % palette.size]
                if (c.optString("id") == selectedId) blockPaint.color = 0xFFFFC850.toInt()
                canvas.drawRoundRect(r, 4f, 4f, blockPaint)

                // Only label when the block is wide enough to read.
                if (w > 48f) {
                    canvas.drawText(c.optString("kind"), r.left + 5f, r.centerY() + 4f, textPaint)
                }
            }
        }

        if (clips.length() == 0) {
            canvas.drawText(
                context.getString(R.string.tl_empty),
                labelWidth + 8f,
                rowHeight,
                emptyPaint
            )
        }
    }

    private var selectedId: String? = null

    fun setSelected(id: String?) {
        selectedId = id
        invalidate()
    }

    /** Height the view needs, so the layout can reserve it. */
    fun desiredHeight(px: Int): Int {
        val rows = max(trackCount(), 1)
        return (rows * (rowHeight + gap)).toInt() + 8
    }

    override fun onTouchEvent(event: MotionEvent): Boolean {
        if (event.action != MotionEvent.ACTION_UP) return true
        for (i in blocks.indices.reversed()) {
            if (blocks[i].contains(event.x, event.y)) {
                performClick()
                onPick?.invoke(blockIds[i])
                return true
            }
        }
        return true
    }

    override fun performClick(): Boolean {
        super.performClick()
        return true
    }
}