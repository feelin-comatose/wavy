package app.wavy.android.data

import app.wavy.android.net.Http
import app.wavy.android.yt.YouTube
import kotlinx.coroutines.flow.MutableStateFlow
import okhttp3.Request
import java.io.File

/**
 * Треки YouTube для прослушивания без интернета: звук скачивается в память приложения
 * (files/offline/<videoId>.m4a) и дальше играет из файла.
 */
object Offline {
    private var dir: File? = null
    /** id скачанных видео — чтобы экран сразу видел изменения */
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

    /** Скачать звук трека. Кусками по 4 МБ: целиком googlevideo отдаёт очень медленно. */
    fun download(t: Track) {
        val id = t.videoId ?: return
        if (file(id) != null) return
        val d = dir ?: return
        fetch(YouTube.audioStream(id), File(d, "$id.part"))
        File(d, "$id.part").renameTo(File(d, "$id.m4a"))
        refresh()
    }

    /** Скачать адрес в файл кусками (Range). */
    fun fetch(url: String, part: File, chunk: Long = 4L * 1024 * 1024) {
        part.outputStream().use { out ->
            var pos = 0L
            while (true) {
                val req = Request.Builder().url(url).header("User-Agent", Http.UA).header("Range", "bytes=$pos-${pos + chunk - 1}").build()
                val total = Http.client.newCall(req).execute().use { r ->
                    if (r.code != 206 && r.code != 200) throw java.io.IOException("YouTube ответил ${r.code}")
                    val body = r.body ?: throw java.io.IOException("пустой ответ")
                    val n = body.byteStream().copyTo(out)
                    pos += n
                    // 200 — сервер отдал всё сразу; иначе общий размер из Content-Range
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
