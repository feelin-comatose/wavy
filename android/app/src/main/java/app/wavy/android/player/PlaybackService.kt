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
import androidx.media3.exoplayer.DefaultLoadControl
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
 * Background playback: ExoPlayer + MediaSession. Provides a notification with buttons and control
 * from the lock screen, headphones and watches. While music plays, the service runs in the foreground.
 */
@androidx.annotation.OptIn(androidx.media3.common.util.UnstableApi::class)
class PlaybackService : MediaSessionService() {
    private var session: MediaSession? = null
    private var bubble: FloatingBubble? = null
    private var errorsInRow = 0

    override fun onCreate() {
        super.onCreate()
        Store.init(this)
        val http = OkHttpDataSource.Factory(Http.client).setUserAgent(Http.UA)
        val base = DefaultDataSource.Factory(this, http)
        val resolving = ResolvingDataSource.Factory(base) { spec -> Media.resolve(spec) }
        // faster start: begin playing after buffering 1 s of audio (default is 2.5 s)
        val loadControl = DefaultLoadControl.Builder()
            .setBufferDurationsMs(15_000, 120_000, 1_000, 2_000)
            .build()
        val player = ExoPlayer.Builder(this)
            .setMediaSourceFactory(DefaultMediaSourceFactory(MediaCache.Factory(this, base, resolving)))
            .setLoadControl(loadControl)
            .setAudioAttributes(AudioAttributes.Builder().setUsage(C.USAGE_MEDIA).setContentType(C.AUDIO_CONTENT_TYPE_MUSIC).build(), true)
            .setHandleAudioBecomingNoisy(true) // headphones unplugged — pause
            .setWakeMode(C.WAKE_MODE_NETWORK)
            .build()
        player.addListener(object : Player.Listener {
            override fun onPlayerError(error: PlaybackException) {
                val cause = error.cause
                // the YouTube link expired — forget it and try again
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
                // a MediaItem from the controller arrives without an address — restore it by track id
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
