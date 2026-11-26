package app.wavy.android.player

import android.net.Uri
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.datasource.DataSpec
import app.wavy.android.data.Track
import app.wavy.android.yt.YouTube

/** Track ↔ MediaItem. YouTube/streams get a wavy://… address; the real stream is substituted on load. */
object Media {
    fun uriOf(t: Track): String = when {
        t.isLocal -> t.path ?: ""
        // internet radio: its own scheme — resolved when played, never cached (the stream is endless)
        t.type == "radio" -> "wavyradio://" + Uri.encode(t.url ?: "")
        // downloaded for offline — play from the file
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

    /** Key for YouTube.audioStream: videoId or the original link. */
    fun streamKey(uri: Uri): String? = when {
        uri.scheme != "wavy" -> null
        uri.host == "yt" -> uri.lastPathSegment
        uri.host == "url" -> Uri.decode(uri.encodedPath?.removePrefix("/") ?: "")
        else -> null
    }

    /** First stream link inside a .pls / .m3u playlist (same as core/src/radio.rs parse_playlist). */
    fun radioPlaylistEntry(text: String): String? = text.lineSequence().map { it.trim() }.firstNotNullOfOrNull { l ->
        val u = if (l.substringBefore('=').lowercase().startsWith("file") && l.contains('=')) l.substringAfter('=').trim() else l
        u.takeIf { it.startsWith("http://") || it.startsWith("https://") }
    }

    /** Radio link → the audio stream: .pls/.m3u station links are followed (up to 3 levels). */
    private fun radioStream(url: String): String {
        var u = url
        repeat(3) {
            val lower = u.lowercase().substringBefore('?')
            if (!(lower.endsWith(".pls") || lower.endsWith(".m3u") || lower.endsWith(".m3u8"))) return u
            u = runCatching { app.wavy.android.net.Http.get(u) }.getOrNull()?.let(::radioPlaylistEntry) ?: return u
        }
        return u
    }

    /** Replace wavy://… with the direct stream URL (called by ExoPlayer on a background thread). */
    fun resolve(spec: DataSpec): DataSpec {
        if (spec.uri.scheme == "wavyradio") return spec.withUri(Uri.parse(radioStream(Uri.decode(spec.uri.toString().removePrefix("wavyradio://")))))
        val key = streamKey(spec.uri) ?: return spec
        return spec.withUri(Uri.parse(YouTube.audioStream(key)))
    }
}
