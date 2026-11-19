package app.wavy.android.player

import android.app.PendingIntent
import android.content.Intent
import androidx.media3.common.AudioAttributes
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.datasource.DefaultDataSource
import androidx.media3.datasource.HttpDataSource
import androidx.media3.datasource.ResolvingDataSource
import androidx.media3.datasource.okhttp.OkHttpDataSource
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.source.DefaultMediaSourceFactory
import androidx.media3.session.MediaSession
import androidx.media3.session.MediaSessionService
import app.wavy.android.MainActivity
import app.wavy.android.data.Store
import app.wavy.android.net.Http
import app.wavy.android.overlay.FloatingBubble
import app.wavy.android.yt.YouTube
import com.google.common.util.concurrent.Futures
import com.google.common.util.concurrent.ListenableFuture

/**
 * Фоновое воспроизведение: ExoPlayer + MediaSession. Даёт уведомление с кнопками, управление
 * с экрана блокировки, наушников и часов. Пока играет музыка, сервис работает на переднем плане.
 */
class PlaybackService : MediaSessionService() {
    private var session: MediaSession? = null
    private var bubble: FloatingBubble? = null
    private var errorsInRow = 0

    override fun onCreate() {
        super.onCreate()
        Store.init(this)
        val http = OkHttpDataSource.Factory(Http.client).setUserAgent(Http.UA)
        val resolving = ResolvingDataSource.Factory(DefaultDataSource.Factory(this, http)) { spec -> Media.resolve(spec) }
        val player = ExoPlayer.Builder(this)
            .setMediaSourceFactory(DefaultMediaSourceFactory(resolving))
            .setAudioAttributes(AudioAttributes.Builder().setUsage(C.USAGE_MEDIA).setContentType(C.AUDIO_CONTENT_TYPE_MUSIC).build(), true)
            .setHandleAudioBecomingNoisy(true) // выдернули наушники — пауза
            .setWakeMode(C.WAKE_MODE_NETWORK)
            .build()
        player.addListener(object : Player.Listener {
            override fun onPlayerError(error: PlaybackException) {
                val cause = error.cause
                // ссылка YouTube протухла — забываем её и пробуем ещё раз
                if (cause is HttpDataSource.InvalidResponseCodeException && cause.responseCode == 403 && errorsInRow == 0) {
                    player.currentMediaItem?.localConfiguration?.uri?.let(Media::streamKey)?.let(YouTube::invalidate)
                    errorsInRow++
                    player.prepare(); player.play()
                    return
                }
                errorsInRow++
                if (errorsInRow < 5 && player.hasNextMediaItem()) { player.seekToNextMediaItem(); player.prepare(); player.play() }
            }
            override fun onMediaItemTransition(mediaItem: MediaItem?, reason: Int) { if (reason != Player.MEDIA_ITEM_TRANSITION_REASON_PLAYLIST_CHANGED) errorsInRow = 0 }
            override fun onIsPlayingChanged(isPlaying: Boolean) { if (isPlaying) errorsInRow = 0 }
        })
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        session = MediaSession.Builder(this, player)
            .setSessionActivity(open)
            .setCallback(object : MediaSession.Callback {
                // MediaItem от контроллера приходит без адреса — восстанавливаем по id трека
                override fun onAddMediaItems(s: MediaSession, c: MediaSession.ControllerInfo, items: MutableList<MediaItem>): ListenableFuture<MutableList<MediaItem>> =
                    Futures.immediateFuture(items.map { mi -> Store.track(mi.mediaId)?.let(Media::item) ?: mi }.toMutableList())
            })
            .build()
        bubble = FloatingBubble(this, player)
    }

    override fun onGetSession(controllerInfo: MediaSession.ControllerInfo): MediaSession? = session

    override fun onTaskRemoved(rootIntent: Intent?) {
        val p = session?.player
        if (p == null || !p.playWhenReady || p.mediaItemCount == 0) stopSelf()
    }

    override fun onDestroy() {
        bubble?.release()
        session?.run { player.release(); release() }
        session = null
        super.onDestroy()
    }
}
