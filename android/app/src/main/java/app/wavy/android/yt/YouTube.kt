package app.wavy.android.yt

import app.wavy.android.data.L
import app.wavy.android.data.Track
import app.wavy.android.net.NewPipeDownloader
import org.schabi.newpipe.extractor.InfoItem
import org.schabi.newpipe.extractor.NewPipe
import org.schabi.newpipe.extractor.Page
import org.schabi.newpipe.extractor.ServiceList
import org.schabi.newpipe.extractor.StreamingService.LinkType
import org.schabi.newpipe.extractor.playlist.PlaylistInfo
import org.schabi.newpipe.extractor.search.SearchInfo
import org.schabi.newpipe.extractor.services.youtube.linkHandler.YoutubeSearchQueryHandlerFactory
import org.schabi.newpipe.extractor.stream.DeliveryMethod
import org.schabi.newpipe.extractor.stream.StreamInfo
import org.schabi.newpipe.extractor.stream.StreamInfoItem
import java.security.MessageDigest
import java.util.concurrent.ConcurrentHashMap

/** YouTube / YouTube Music / SoundCloud … через NewPipeExtractor. Всё блокирующее — звать не из UI-потока. */
object YouTube {
    @Volatile private var ready = false
    fun init() { if (!ready) synchronized(this) { if (!ready) { NewPipe.init(NewPipeDownloader()); ready = true } } }

    class Import(val name: String?, val source: String, val tracks: List<Track>)

    private fun videoIdOf(url: String): String? = runCatching {
        ServiceList.YouTube.streamLHFactory.getId(url)
    }.getOrNull()?.takeIf { it.length == 11 }

    private fun hash(s: String) = MessageDigest.getInstance("SHA-1").digest(s.toByteArray()).joinToString("") { "%02x".format(it) }.take(16)

    fun cleanArtist(s: String) = s.removeSuffix(" - Topic").trim()

    /** Элемент поиска/плейлиста → трек (id совпадают с десктопом: yt_<videoId>). */
    fun toTrack(item: StreamInfoItem, music: Boolean = false, addedAt: Long = System.currentTimeMillis()): Track {
        val vid = videoIdOf(item.url)
        return if (vid != null) Track(
            id = "yt_$vid", type = "youtube", videoId = vid,
            url = (if (music) "https://music.youtube.com/watch?v=" else "https://www.youtube.com/watch?v=") + vid,
            title = item.name, artist = cleanArtist(item.uploaderName ?: "").ifEmpty { "YouTube" },
            duration = item.duration.toDouble().coerceAtLeast(0.0),
            cover = "https://i.ytimg.com/vi/$vid/hqdefault.jpg", addedAt = addedAt,
        ) else Track(
            id = "ext_${hash(item.url)}", type = "stream", url = item.url, title = item.name,
            artist = cleanArtist(item.uploaderName ?: "").ifEmpty { "Stream" }, duration = item.duration.toDouble().coerceAtLeast(0.0),
            cover = item.thumbnails.maxByOrNull { it.height }?.url, addedAt = addedAt,
        )
    }

    /** Поиск: music=true — песни из YouTube Music (чище результаты), иначе обычные видео. */
    fun search(query: String, music: Boolean = true, limit: Int = 25): List<Track> {
        init()
        val yt = ServiceList.YouTube
        fun run(filter: String) = SearchInfo.getInfo(yt, yt.searchQHFactory.fromQuery(query, listOf(filter), ""))
            .relatedItems.filterIsInstance<StreamInfoItem>()
            .filter { it.streamType.name != "LIVE_STREAM" && it.streamType.name != "AUDIO_LIVE_STREAM" }
        if (music) {
            // раздел «песни» YouTube Music иногда отдаёт пусто — тогда ищем по обычным видео
            val songs = runCatching { run(YoutubeSearchQueryHandlerFactory.MUSIC_SONGS) }.getOrDefault(emptyList())
            if (songs.isNotEmpty()) return songs.take(limit).map { toTrack(it, true) }
        }
        return run(YoutubeSearchQueryHandlerFactory.VIDEOS).take(limit).map { toTrack(it, music) }
    }

    /** Плейлист / альбом / видео по ссылке (YouTube, YouTube Music, SoundCloud, Bandcamp…). */
    fun importUrl(url: String, progress: (String) -> Unit = {}): Import {
        init()
        val service = NewPipe.getServiceByUrl(url)
        val music = url.contains("music.youtube.com")
        val base = System.currentTimeMillis()
        return when (service.getLinkTypeByUrl(url)) {
            LinkType.PLAYLIST -> {
                progress(L("Читаю плейлист…", "Reading the playlist…"))
                val info = PlaylistInfo.getInfo(service, url)
                val items = ArrayList<StreamInfoItem>(info.relatedItems)
                var next: Page? = info.nextPage
                while (next != null && items.size < 5000) {
                    val page = PlaylistInfo.getMoreItems(service, url, next)
                    items += page.items
                    next = page.nextPage
                    progress(L("Читаю плейлист… ${items.size}", "Reading the playlist… ${items.size}"))
                }
                val tracks = items.filter { it.name != "[Private video]" && it.name != "[Deleted video]" }
                    .mapIndexed { i, it -> toTrack(it, music, base + (items.size - i)) }
                Import(info.name, url, tracks.distinctBy { it.id })
            }
            LinkType.STREAM -> {
                val s = StreamInfo.getInfo(service, url)
                val vid = videoIdOf(s.url)
                val t = if (vid != null) Track(
                    id = "yt_$vid", type = "youtube", videoId = vid, url = url, title = s.name,
                    artist = cleanArtist(s.uploaderName ?: ""), duration = s.duration.toDouble(),
                    cover = "https://i.ytimg.com/vi/$vid/hqdefault.jpg", addedAt = base,
                ) else Track(id = "ext_${hash(url)}", type = "stream", url = url, title = s.name, artist = cleanArtist(s.uploaderName ?: ""),
                    duration = s.duration.toDouble(), cover = s.thumbnails.maxByOrNull { it.height }?.url, addedAt = base)
                Import(null, url, listOf(t))
            }
            else -> throw IllegalArgumentException(L("Эта ссылка не похожа на плейлист или трек", "This link doesn't look like a playlist or a track"))
        }
    }

    /** Радио по треку: микс YouTube (похожие песни), без самого трека. */
    fun radio(videoId: String, limit: Int = 30): List<Track> {
        init()
        val yt = ServiceList.YouTube
        val info = PlaylistInfo.getInfo(yt, "https://www.youtube.com/watch?v=$videoId&list=RD$videoId")
        val base = System.currentTimeMillis()
        return info.relatedItems.map { toTrack(it, true, base) }.filter { it.videoId != videoId }.distinctBy { it.id }.take(limit)
    }

    // ---- потоки для плеера (кэшируются: ссылки живут ~6 часов) ----
    private class Cached(val url: String, val until: Long)
    private val audioCache = ConcurrentHashMap<String, Cached>()
    private val videoCache = ConcurrentHashMap<String, Cached>()

    fun invalidate(key: String) { audioCache.remove(key); videoCache.remove(key) }

    private fun pageUrl(key: String) = if (key.startsWith("http")) key else "https://www.youtube.com/watch?v=$key"

    /** Прямая ссылка на лучший аудиопоток (m4a в приоритете — играет везде). */
    fun audioStream(key: String): String {
        audioCache[key]?.takeIf { it.until > System.currentTimeMillis() }?.let { return it.url }
        init()
        val info = StreamInfo.getInfo(pageUrl(key))
        val a = info.audioStreams.filter { it.isUrl && it.deliveryMethod == DeliveryMethod.PROGRESSIVE_HTTP }
            .sortedWith(compareByDescending<org.schabi.newpipe.extractor.stream.AudioStream> { it.format?.name == "M4A" }.thenByDescending { it.averageBitrate })
            .firstOrNull() ?: throw IllegalStateException(L("у видео нет аудиопотока", "the video has no audio stream"))
        audioCache[key] = Cached(a.content, System.currentTimeMillis() + 5 * 3600_000)
        return a.content
    }

    /** Видео без звука (≤ 720p) для показа клипа синхронно со звуком. */
    fun videoStream(key: String): String {
        videoCache[key]?.takeIf { it.until > System.currentTimeMillis() }?.let { return it.url }
        init()
        val info = StreamInfo.getInfo(pageUrl(key))
        val res = { s: String -> s.filter { it.isDigit() }.take(4).toIntOrNull() ?: 0 }
        val v = info.videoOnlyStreams.filter { it.isUrl && it.deliveryMethod == DeliveryMethod.PROGRESSIVE_HTTP && res(it.getResolution()) in 1..720 }
            .sortedWith(compareByDescending<org.schabi.newpipe.extractor.stream.VideoStream> { res(it.getResolution()) }.thenByDescending { it.format?.name == "MPEG_4" })
            .firstOrNull()
            ?: info.videoStreams.firstOrNull { it.isUrl } // со звуком — тоже сгодится (звук приглушим)
            ?: throw IllegalStateException(L("у видео нет видеопотока", "the video has no video stream"))
        videoCache[key] = Cached(v.content, System.currentTimeMillis() + 5 * 3600_000)
        return v.content
    }
}
