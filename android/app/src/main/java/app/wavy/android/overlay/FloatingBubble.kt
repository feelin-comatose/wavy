package app.wavy.android.overlay

import android.annotation.SuppressLint
import android.content.Context
import android.content.Intent
import android.graphics.Color
import android.graphics.PixelFormat
import android.graphics.drawable.GradientDrawable
import android.provider.Settings
import android.text.TextUtils
import android.util.TypedValue
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.WindowManager
import android.widget.ImageButton
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.TextView
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.ProcessLifecycleOwner
import androidx.media3.common.MediaMetadata
import androidx.media3.common.Player
import app.wavy.android.MainActivity
import app.wavy.android.data.Store
import coil3.ImageLoader
import coil3.request.ImageRequest
import coil3.request.target
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch

/**
 * Плавающий мини-плеер поверх других приложений (аналог оверлея на ПК).
 * Показывается, когда приложение свёрнуто, есть что играть и включено в настройках
 * (нужно разрешение «Поверх других приложений»). Перетаскивается пальцем.
 */
class FloatingBubble(private val ctx: Context, private val player: Player) {
    private val wm = ctx.getSystemService(Context.WINDOW_SERVICE) as WindowManager
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private var view: View? = null
    private lateinit var cover: ImageView
    private lateinit var title: TextView
    private lateinit var artist: TextView
    private lateinit var playBtn: ImageButton
    private var appVisible = true
    private var enabled = false
    private val images = ImageLoader(ctx)

    private val lifecycle = object : DefaultLifecycleObserver {
        override fun onStart(owner: LifecycleOwner) { appVisible = true; refresh() }
        override fun onStop(owner: LifecycleOwner) { appVisible = false; refresh() }
    }
    private val listener = object : Player.Listener {
        override fun onEvents(p: Player, events: Player.Events) { refresh() }
    }

    init {
        ProcessLifecycleOwner.get().lifecycle.addObserver(lifecycle)
        player.addListener(listener)
        scope.launch { Store.lib.map { it.settings.floatingPlayer }.distinctUntilChanged().collect { enabled = it; refresh() } }
    }

    private fun dp(v: Int) = TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, v.toFloat(), ctx.resources.displayMetrics).toInt()

    private fun refresh() {
        val want = enabled && !appVisible && player.mediaItemCount > 0 && Settings.canDrawOverlays(ctx)
        if (want && view == null) show()
        if (!want && view != null) hide()
        view?.let { bind(player.mediaMetadata) }
    }

    private fun bind(md: MediaMetadata) {
        title.text = md.title ?: ""
        artist.text = md.artist ?: ""
        playBtn.setImageResource(if (player.isPlaying) android.R.drawable.ic_media_pause else android.R.drawable.ic_media_play)
        val art = md.artworkUri
        if (cover.tag != art) {
            cover.tag = art
            images.enqueue(ImageRequest.Builder(ctx).data(art).target(cover).build())
        }
    }

    @SuppressLint("ClickableViewAccessibility")
    private fun show() {
        val root = LinearLayout(ctx).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(dp(8), dp(8), dp(6), dp(8))
            background = GradientDrawable().apply { setColor(Color.argb(235, 12, 12, 14)); cornerRadius = dp(18).toFloat(); setStroke(dp(1), Color.argb(40, 255, 255, 255)) }
            elevation = dp(8).toFloat()
        }
        cover = ImageView(ctx).apply {
            layoutParams = LinearLayout.LayoutParams(dp(44), dp(44))
            scaleType = ImageView.ScaleType.CENTER_CROP
            clipToOutline = true
            background = GradientDrawable().apply { setColor(Color.rgb(24, 24, 28)); cornerRadius = dp(10).toFloat() }
        }
        val texts = LinearLayout(ctx).apply {
            orientation = LinearLayout.VERTICAL
            layoutParams = LinearLayout.LayoutParams(dp(150), LinearLayout.LayoutParams.WRAP_CONTENT).apply { marginStart = dp(10) }
        }
        title = TextView(ctx).apply { setTextColor(Color.WHITE); textSize = 14f; maxLines = 1; ellipsize = TextUtils.TruncateAt.END; paint.isFakeBoldText = true }
        artist = TextView(ctx).apply { setTextColor(Color.rgb(160, 160, 170)); textSize = 12f; maxLines = 1; ellipsize = TextUtils.TruncateAt.END }
        texts.addView(title); texts.addView(artist)
        fun btn(icon: Int, onClick: () -> Unit) = ImageButton(ctx).apply {
            setImageResource(icon); setBackgroundColor(Color.TRANSPARENT); setColorFilter(Color.WHITE)
            layoutParams = LinearLayout.LayoutParams(dp(40), dp(40)); setOnClickListener { onClick() }
        }
        playBtn = btn(android.R.drawable.ic_media_play) { if (player.isPlaying) player.pause() else player.play() }
        root.addView(cover); root.addView(texts); root.addView(playBtn)
        root.addView(btn(android.R.drawable.ic_media_next) { player.seekToNextMediaItem() })
        root.addView(btn(android.R.drawable.ic_menu_close_clear_cancel) { Store.settings { it.copy(floatingPlayer = false) } })

        val lp = WindowManager.LayoutParams(
            WindowManager.LayoutParams.WRAP_CONTENT, WindowManager.LayoutParams.WRAP_CONTENT,
            WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS,
            PixelFormat.TRANSLUCENT,
        ).apply { gravity = Gravity.TOP or Gravity.START; x = dp(12); y = dp(120) }

        // перетаскивание; короткое касание обложки/текста — открыть приложение
        var sx = 0f; var sy = 0f; var ox = 0; var oy = 0; var moved = false
        val drag = View.OnTouchListener { _, e ->
            when (e.action) {
                MotionEvent.ACTION_DOWN -> { sx = e.rawX; sy = e.rawY; ox = lp.x; oy = lp.y; moved = false; true }
                MotionEvent.ACTION_MOVE -> {
                    val dx = (e.rawX - sx).toInt(); val dy = (e.rawY - sy).toInt()
                    if (kotlin.math.abs(dx) + kotlin.math.abs(dy) > dp(6)) moved = true
                    lp.x = ox + dx; lp.y = oy + dy; view?.let { wm.updateViewLayout(it, lp) }; true
                }
                MotionEvent.ACTION_UP -> {
                    if (!moved) ctx.startActivity(Intent(ctx, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_REORDER_TO_FRONT))
                    true
                }
                else -> false
            }
        }
        root.setOnTouchListener(drag); cover.setOnTouchListener(drag); texts.setOnTouchListener(drag)
        runCatching { wm.addView(root, lp) }.onSuccess { view = root }
    }

    private fun hide() { view?.let { runCatching { wm.removeView(it) } }; view = null }

    fun release() {
        hide()
        player.removeListener(listener)
        ProcessLifecycleOwner.get().lifecycle.removeObserver(lifecycle)
        scope.cancel()
    }
}
