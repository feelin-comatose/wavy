package app.wavy.android.player

import android.net.Uri
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.datasource.DataSpec
import app.wavy.android.data.Track
import app.wavy.android.yt.YouTube

/** Трек ↔ MediaItem. YouTube/стримы получают адрес wavy://…, настоящий поток подставляется при загрузке. */
object Media {
    fun uriOf(t: Track): String = when {
        t.isLocal -> t.path ?: ""
        // скачано для офлайна — играем из файла
        t.videoId != null && app.wavy.android.data.Offline.file(t.videoId) != null -> Uri.fromFile(app.wavy.android.data.Offline.file(t.videoId)).toString()
        t.videoId != null -> "wavy://yt/${t.videoId}"
        else -> "wavy://url/" + Uri.encode(t.url ?: "")
    }

    fun item(t: Track): MediaItem {
        val md = MediaMetadata.Builder()
            .setTitle(t.title).setArtist(t.artist).setAlbumTitle(t.album.ifEmpty { null })
            .setArtworkUri(t.cover?.let(Uri::parse))
            .setIsPlayable(true).setIsBrowsable(false)
            .build()
        return MediaItem.Builder().setMediaId(t.id).setUri(uriOf(t)).setMediaMetadata(md).build()
    }

    /** Ключ для YouTube.audioStream: videoId или исходная ссылка. */
    fun streamKey(uri: Uri): String? = when {
        uri.scheme != "wavy" -> null
        uri.host == "yt" -> uri.lastPathSegment
        uri.host == "url" -> Uri.decode(uri.encodedPath?.removePrefix("/") ?: "")
        else -> null
    }

    /** Подмена wavy://… на прямую ссылку потока (вызывается ExoPlayer в фоновом потоке). */
    fun resolve(spec: DataSpec): DataSpec {
        val key = streamKey(spec.uri) ?: return spec
        return spec.withUri(Uri.parse(YouTube.audioStream(key)))
    }
}
