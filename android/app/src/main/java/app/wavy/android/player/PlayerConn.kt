package app.wavy.android.player

import android.content.ComponentName
import android.content.Context
import androidx.media3.common.Player
import androidx.media3.session.MediaController
import androidx.media3.session.SessionToken
import app.wavy.android.data.Session
import app.wavy.android.data.Store
import com.google.common.util.concurrent.MoreExecutors
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

/** Связь интерфейса с PlaybackService через MediaController. */
object PlayerConn {
    data class State(
        val currentId: String? = null,
        val isPlaying: Boolean = false,
        val buffering: Boolean = false,
        val index: Int = 0,
        val queue: List<String> = emptyList(),
        val shuffle: Boolean = false,
        val repeat: Int = Player.REPEAT_MODE_OFF,
        val error: String? = null,
    )

    var controller: MediaController? = null; private set
    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state

    private val listener = object : Player.Listener {
        override fun onEvents(player: Player, events: Player.Events) { update() }
    }

    fun connect(ctx: Context) {
        if (controller != null) return
        val token = SessionToken(ctx, ComponentName(ctx, PlaybackService::class.java))
        val f = MediaController.Builder(ctx, token).buildAsync()
        f.addListener({
            val c = runCatching { f.get() }.getOrNull() ?: return@addListener
            controller = c
            c.addListener(listener)
            restore(c)
            update()
        }, MoreExecutors.directExecutor())
    }

    private fun update() {
        val c = controller ?: return
        _state.value = State(
            currentId = c.currentMediaItem?.mediaId,
            isPlaying = c.isPlaying,
            buffering = c.playbackState == Player.STATE_BUFFERING,
            index = c.currentMediaItemIndex,
            queue = (0 until c.mediaItemCount).map { c.getMediaItemAt(it).mediaId },
            shuffle = c.shuffleModeEnabled,
            repeat = c.repeatMode,
            error = c.playerError?.let { e -> (e.cause?.message ?: e.errorCodeName) },
        )
    }

    /** Очередь с прошлого раза — на паузе, с той же позиции. */
    private fun restore(c: MediaController) {
        if (c.mediaItemCount > 0) return
        val s = Store.lib.value.session
        val items = s.queue.mapNotNull(Store::track).map(Media::item)
        if (items.isEmpty()) return
        c.setMediaItems(items, s.index.coerceIn(0, items.lastIndex), s.positionMs)
        c.shuffleModeEnabled = Store.lib.value.settings.shuffle
        c.prepare()
    }

    fun saveSession() {
        val c = controller ?: return
        if (c.mediaItemCount == 0) return
        Store.session(Session((0 until c.mediaItemCount).map { c.getMediaItemAt(it).mediaId }, c.currentMediaItemIndex, c.currentPosition))
    }

    /** Для скриншот-тестов: подставить состояние без настоящего плеера. */
    fun fakeState(s: State) { _state.value = s }

    fun position() = controller?.currentPosition ?: 0L
    fun duration() = controller?.duration?.takeIf { it > 0 } ?: 0L

    fun play(ids: List<String>, start: Int = 0, shuffle: Boolean = false) {
        val c = controller ?: return
        val items = ids.mapNotNull(Store::track).map(Media::item)
        if (items.isEmpty()) return
        c.shuffleModeEnabled = shuffle || Store.lib.value.settings.shuffle
        c.setMediaItems(items, start.coerceIn(0, items.lastIndex), 0)
        c.prepare(); c.play()
    }

    fun playNext(ids: List<String>) {
        val c = controller ?: return
        if (c.mediaItemCount == 0) return play(ids)
        c.addMediaItems(c.currentMediaItemIndex + 1, ids.mapNotNull(Store::track).map(Media::item))
    }

    fun enqueue(ids: List<String>) {
        val c = controller ?: return
        if (c.mediaItemCount == 0) return play(ids)
        c.addMediaItems(ids.mapNotNull(Store::track).map(Media::item))
    }

    fun toggle() { controller?.let { if (it.isPlaying) it.pause() else { if (it.playbackState == Player.STATE_IDLE) it.prepare(); it.play() } } }
    fun next() { controller?.seekToNextMediaItem() }
    fun prev() { controller?.seekToPrevious() }
    fun seek(ms: Long) { controller?.seekTo(ms) }
    fun jump(index: Int) { controller?.let { it.seekTo(index, 0); it.play() } }

    fun setShuffle(on: Boolean) { controller?.shuffleModeEnabled = on; Store.settings { it.copy(shuffle = on) } }
    fun cycleRepeat() {
        val c = controller ?: return
        c.repeatMode = when (c.repeatMode) { Player.REPEAT_MODE_OFF -> Player.REPEAT_MODE_ALL; Player.REPEAT_MODE_ALL -> Player.REPEAT_MODE_ONE; else -> Player.REPEAT_MODE_OFF }
        Store.settings { it.copy(repeat = when (c.repeatMode) { Player.REPEAT_MODE_ALL -> "all"; Player.REPEAT_MODE_ONE -> "one"; else -> "off" }) }
    }
}
