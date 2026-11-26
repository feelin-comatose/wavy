package app.wavy.android.data

import app.wavy.android.net.Http
import app.wavy.android.yt.YouTube
import kotlinx.coroutines.flow.MutableStateFlow
import okhttp3.Request
import java.io.File

/**
 * YouTube tracks for offline listening: the audio is downloaded into the app's storage
 * (files/offline/<videoId>.m4a) and then plays from the file.
 */
object Offline {
    private var dir: File? = null
    /** ids of downloaded videos — so the screen sees changes immediately */
    val saved = MutableStateFlow<Set<String>>(emptySet())

    fun init(filesDir: File) {
        dir = File(filesDir, "offline").apply { mkdirs() }
        refresh()
    }

    private fun refresh() {
        saved.value = dir?.listFiles()?.filter { it.name.endsWith(".m4a") }?.map { it.name.removeSuffix(".m4a") }?.toSet() ?: emptySet()
    }

    fun file(videoId: String): File? = dir?.let { File(it, "$videoId.m4a") }?.takeIf { it.isFile && it.length() > 0 }

    fun has(t: Track) = t.videoId != null && t.videoId in saved.value

    /** Download a track's audio. In 4 MB chunks: googlevideo serves the whole file very slowly. */
    fun download(t: Track) {
        val id = t.videoId ?: return
        if (file(id) != null) return
        val d = dir ?: return
        fetch(YouTube.audioStream(id), File(d, "$id.part"))
        File(d, "$id.part").renameTo(File(d, "$id.m4a"))
        refresh()
    }

    /** Download a URL to a file in chunks (Range). */
    fun fetch(url: String, part: File, chunk: Long = 4L * 1024 * 1024) {
        part.outputStream().use { out ->
            var pos = 0L
            while (true) {
                val req = Request.Builder().url(url).header("User-Agent", Http.UA).header("Range", "bytes=$pos-${pos + chunk - 1}").build()
                val total = Http.client.newCall(req).execute().use { r ->
                    if (r.code != 206 && r.code != 200) throw java.io.IOException(L("YouTube ответил ${r.code}", "YouTube replied ${r.code}"))
                    val body = r.body ?: throw java.io.IOException(L("пустой ответ", "empty response"))
                    val n = body.byteStream().copyTo(out)
                    pos += n
                    // 200 — the server sent everything at once; otherwise the total size comes from Content-Range
                    if (r.code == 200) pos else r.header("Content-Range")?.substringAfter('/')?.toLongOrNull() ?: pos
                }
                if (pos >= total) break
            }
        }
    }

    fun remove(t: Track) { t.videoId?.let { file(it)?.delete() }; refresh() }

    fun clear() { dir?.listFiles()?.forEach { it.delete() }; refresh() }

    fun size(): Long = dir?.listFiles()?.sumOf { it.length() } ?: 0L
}
