package app.wavy.android.ui

import android.content.Context
import app.wavy.android.data.AnimeParse
import app.wavy.android.data.LocalScanner
import app.wavy.android.data.Shazam
import app.wavy.android.data.Shikimori
import okhttp3.MediaType.Companion.toMediaTypeOrNull
import app.wavy.android.data.Store
import app.wavy.android.data.Sync
import app.wavy.android.data.Track
import app.wavy.android.update.Updater
import app.wavy.android.yt.Spotify
import app.wavy.android.yt.YouTube
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File

/** Фоновые операции и состояние экрана, которое переживает пересоздание Activity. */
object AppState {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)

    /** Всплывающее сообщение внизу экрана. */
    val message = MutableStateFlow<String?>(null)
    val busy = MutableStateFlow<String?>(null)

    val searchQuery = MutableStateFlow("")
    val searchMusic = MutableStateFlow(true)
    val searchResults = MutableStateFlow<List<Track>>(emptyList())
    val searching = MutableStateFlow(false)

    sealed interface Upd { data object Idle : Upd; data object Checking : Upd; data object UpToDate : Upd
        data class Available(val rel: Updater.Release) : Upd; data class Downloading(val pct: Int) : Upd
        data class Ready(val file: File) : Upd; data class Error(val msg: String) : Upd }
    val update = MutableStateFlow<Upd>(Upd.Idle)

    fun say(m: String) { message.value = m }

    private fun bg(label: String, block: suspend () -> Unit) = scope.launch {
        busy.value = label
        try { block() } catch (e: Throwable) { say("Ошибка: ${e.message ?: e.javaClass.simpleName}") } finally { busy.value = null }
    }

    fun scanLocal(ctx: Context) = bg("Ищу музыку и видео на телефоне…") {
        val found = withContext(Dispatchers.IO) { LocalScanner.scan(ctx) }
        Store.addTracks(found)
        // серии аниме: видео с номером серии в имени → тайтлы, обложки и названия с Шикимори
        val ids = found.filter { it.video }.map { it.id }.toSet()
        val pending = Store.addEpisodes(LocalScanner.videoNames.filterKeys { it in ids })
        say(if (found.isEmpty()) "На телефоне не нашлось музыки" else "Найдено: ${found.count { !it.video }} треков, ${found.count { it.video }} видео")
        if (pending.isNotEmpty()) matchAnime(pending)
    }

    /** Найти тайтлы на Шикимори. Видео с одной «серией», которых там нет, — не аниме: возвращаем в библиотеку. */
    fun matchAnime(list: List<app.wavy.android.data.Anime>) = bg("Ищу аниме на Шикимори…") {
        for (a in list) {
            val p = AnimeParse.Parsed(a.title, null, if ("#s" in a.key) a.key.substringAfter("#s").toIntOrNull() else null)
            val hit = runCatching { withContext(Dispatchers.IO) { Shikimori.bestMatch(a.title, p.season, a.episodes.size, Shikimori.search(AnimeParse.searchQuery(p))) } }.getOrNull()
            if (hit == null) {
                if (a.episodes.size < 2) Store.deleteAnime(a.id) else Store.editAnime(a.id) { it.copy(matched = true) }
                continue
            }
            Store.editAnime(a.id) { it.copy(shikiId = hit.id, name = hit.name, russian = hit.russian, kind = hit.kind, episodesTotal = hit.episodes, score = hit.score, year = hit.year, poster = hit.poster, url = hit.url, matched = true) }
            val title = hit.russian.ifEmpty { hit.name }
            Store.addTracks(a.episodes.mapNotNull { Store.track(it) }.map { it.copy(artist = title, cover = hit.poster ?: it.cover) })
        }
    }

    /** Смотреть тайтл: с последней недосмотренной серии (или с выбранной). */
    fun playAnime(id: String, episode: String? = null) {
        val a = Store.anime(id) ?: return
        if (a.episodes.isEmpty()) return
        val start = episode?.let { a.episodes.indexOf(it) }?.takeIf { it >= 0 }
            ?: a.last?.let { l -> a.episodes.indexOf(l).takeIf { it >= 0 }?.let { i -> if (l in a.watched && i + 1 < a.episodes.size) i + 1 else i } }
            ?: a.episodes.indexOfFirst { it !in a.watched }.coerceAtLeast(0)
        app.wavy.android.player.PlayerConn.play(a.episodes, start)
    }

    // ---------- синхронизация с ПК по Wi-Fi ----------
    /** Скачать треки для прослушивания без интернета. */
    fun downloadOffline(ids: List<String>) = bg("Скачиваю для офлайна…") {
        val tracks = ids.mapNotNull { Store.track(it) }.filter { it.videoId != null && !app.wavy.android.data.Offline.has(it) }
        if (tracks.isEmpty()) { say("Всё уже скачано"); return@bg }
        var ok = 0
        var failed = 0
        for ((i, t) in tracks.withIndex()) {
            busy.value = "Скачиваю для офлайна: ${i + 1} из ${tracks.size} · ${t.title}"
            try { withContext(Dispatchers.IO) { app.wavy.android.data.Offline.download(t) }; ok++ } catch (e: Exception) { failed++ }
        }
        say("Скачано для офлайна: $ok" + if (failed > 0) ", не вышло: $failed" else "")
    }

    /** Открыт ли пульт ПК. */
    val remoteOpen = MutableStateFlow(false)

    fun wifiSync(addr: String, pin: String) = bg("Синхронизация с ПК…") {
        val base = "http://" + addr.trim().removePrefix("http://").trimEnd('/').let { if (':' in it) it else "$it:3738" } + "/wavy/sync?pin=" + pin.trim()
        val r = withContext(Dispatchers.IO) {
            val fromPc = app.wavy.android.net.Http.get(base)
            val got = Sync.import(fromPc)
            val req = okhttp3.Request.Builder().url(base).post(okhttp3.RequestBody.create("application/json".toMediaTypeOrNull(), Sync.export())).build()
            app.wavy.android.net.Http.client.newCall(req).execute().use { resp -> if (!resp.isSuccessful) throw java.io.IOException(if (resp.code == 403) "неверный PIN" else "ПК ответил ${resp.code}") }
            got
        }
        say("С ПК: ${r.tracks} треков, новых плейлистов ${r.newPlaylists}. Плейлисты телефона отправлены на ПК")
    }

    // ---------- Shazam ----------
    sealed interface Shz { data object Idle : Shz; data object Listening : Shz; data class Found(val f: Shazam.Found) : Shz; data object NotFound : Shz; data class Error(val msg: String) : Shz; data object Adding : Shz }
    val shazam = MutableStateFlow<Shz>(Shz.Idle)
    val shazamOpen = MutableStateFlow(false)
    /** Activity спрашивает разрешение на микрофон и запускает listen(). */
    var micRequest: (() -> Unit)? = null
    fun startShazam() { shazamOpen.value = true; micRequest?.invoke() ?: listen() }

    /** Слушать микрофон 12 секунд и спросить Shazam. Разрешение на микрофон проверяет Activity. */
    @android.annotation.SuppressLint("MissingPermission")
    fun listen() = scope.launch {
        shazam.value = Shz.Listening
        shazam.value = try {
            val samples = withContext(Dispatchers.IO) { recordMic(Shazam.SECONDS) }
            withContext(Dispatchers.IO) { Shazam.recognize(samples) }?.let { Shz.Found(it) } ?: Shz.NotFound
        } catch (e: Throwable) { Shz.Error(e.message ?: "не получилось") }
    }

    @android.annotation.SuppressLint("MissingPermission")
    private fun recordMic(secs: Int): FloatArray {
        val rate = Shazam.RATE
        val min = android.media.AudioRecord.getMinBufferSize(rate, android.media.AudioFormat.CHANNEL_IN_MONO, android.media.AudioFormat.ENCODING_PCM_16BIT)
        val rec = android.media.AudioRecord(android.media.MediaRecorder.AudioSource.MIC, rate, android.media.AudioFormat.CHANNEL_IN_MONO, android.media.AudioFormat.ENCODING_PCM_16BIT, maxOf(min, rate))
        if (rec.state != android.media.AudioRecord.STATE_INITIALIZED) { rec.release(); throw IllegalStateException("микрофон недоступен") }
        val buf = ShortArray(rate * secs)
        rec.startRecording()
        try {
            var got = 0
            while (got < buf.size) { val n = rec.read(buf, got, buf.size - got); if (n <= 0) break; got += n }
        } finally { rec.stop(); rec.release() }
        return FloatArray(buf.size) { buf[it] / 32768f }
    }

    /** Добавить найденное: трек ищется на YouTube, попадает в библиотеку и плейлист «Опенинги». */
    fun addFound(f: Shazam.Found) = scope.launch {
        shazam.value = Shz.Adding
        try {
            val t = withContext(Dispatchers.IO) { YouTube.search("${f.artist} ${f.title}", music = true, limit = 1).firstOrNull() } ?: throw IllegalStateException("на YouTube не нашлось")
            Store.addTracks(listOf(t))
            val pl = Store.lib.value.playlists.firstOrNull { it.name == "Опенинги" }?.id ?: Store.createPlaylist("Опенинги").id
            Store.addToPlaylist(pl, listOf(t.id))
            say("«${f.title}» — в библиотеке и плейлисте «Опенинги»")
            shazam.value = Shz.Idle
        } catch (e: Throwable) { shazam.value = Shz.Error(e.message ?: "не получилось добавить") }
    }

    /** Ссылка → треки (Spotify / YouTube / YouTube Music / SoundCloud…). onDone(playlistId). */
    fun importUrl(url: String, intoPlaylist: String? = null, onDone: (String?) -> Unit = {}) = bg("Импорт…") {
        val progress: (String) -> Unit = { m -> scope.launch { busy.value = m } }
        val s = Store.lib.value.settings
        val (name, source, tracks, note) = withContext(Dispatchers.IO) {
            if (Spotify.isSpotify(url)) {
                val r = Spotify.import(url, s.spotifyClientId, s.spotifyClientSecret, progress)
                var note = ""
                if (r.notFound.isNotEmpty()) note += ", не нашлось на YouTube: ${r.notFound.size}"
                if (r.truncated) note += ". Без ключа Spotify — только первые 100 треков"
                Quad(if (r.kind == "track") null else r.name, r.source, r.tracks, note)
            } else {
                val r = YouTube.importUrl(url.trim(), progress)
                Quad(r.name, r.source, r.tracks, "")
            }
        }
        Store.addTracks(tracks)
        val ids = tracks.map { it.id }
        val pid = when {
            intoPlaylist != null -> { Store.addToPlaylist(intoPlaylist, ids); intoPlaylist }
            name != null -> Store.createPlaylist(name, ids, source).id
            else -> null
        }
        say("Добавлено ${tracks.size} трек(ов)$note")
        onDone(pid)
    }

    fun syncPlaylist(id: String) {
        val p = Store.lib.value.playlists.firstOrNull { it.id == id } ?: return
        val src = p.source?.takeIf { it.startsWith("http") } ?: return say("У плейлиста нет ссылки-источника")
        bg("Синхронизирую «${p.name}»…") {
            val s = Store.lib.value.settings
            val tracks = withContext(Dispatchers.IO) {
                if (Spotify.isSpotify(src)) Spotify.import(src, s.spotifyClientId, s.spotifyClientSecret).tracks else YouTube.importUrl(src).tracks
            }
            Store.addTracks(tracks)
            Store.setPlaylistTracks(id, tracks.map { it.id })
            say("Синхронизировано: ${tracks.size} треков")
        }
    }

    fun search() {
        val q = searchQuery.value.trim()
        if (q.isEmpty()) return
        if (q.startsWith("http")) { importUrl(q); return }
        scope.launch {
            searching.value = true
            try { searchResults.value = withContext(Dispatchers.IO) { YouTube.search(q, searchMusic.value) } }
            catch (e: Throwable) { say("Поиск не удался: ${e.message}") }
            finally { searching.value = false }
        }
    }

    fun importExport(text: String) = bg("Импорт плейлистов…") {
        val r = withContext(Dispatchers.IO) { Sync.import(text) }
        say("Импортировано: ${r.tracks} треков, новых плейлистов ${r.newPlaylists}, обновлено ${r.updatedPlaylists}" + if (r.skippedLocal > 0) " (локальные файлы с ПК не переносятся: ${r.skippedLocal})" else "")
    }

    fun checkUpdate(manual: Boolean) = scope.launch {
        update.value = Upd.Checking
        update.value = try {
            withContext(Dispatchers.IO) { Updater.check() }?.let { Upd.Available(it) } ?: Upd.UpToDate
        } catch (e: Throwable) { if (manual) Upd.Error(e.message ?: "не удалось проверить") else Upd.Idle }
    }

    fun downloadUpdate(ctx: Context, rel: Updater.Release) = scope.launch {
        update.value = Upd.Downloading(0)
        update.value = try {
            val f = withContext(Dispatchers.IO) { Updater.download(ctx, rel) { p -> update.value = Upd.Downloading(p) } }
            Upd.Ready(f)
        } catch (e: Throwable) { Upd.Error(e.message ?: "не удалось скачать") }
    }

    private data class Quad(val name: String?, val source: String, val tracks: List<Track>, val note: String)
}
