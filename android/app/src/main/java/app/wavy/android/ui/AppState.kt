package app.wavy.android.ui

import app.wavy.android.data.L
import android.content.Context
import app.wavy.android.data.AnimeParse
import app.wavy.android.data.LocalScanner
import app.wavy.android.data.Shazam
import app.wavy.android.data.Shikimori
import okhttp3.MediaType.Companion.toMediaTypeOrNull
import app.wavy.android.data.Store
import app.wavy.android.player.PlayerConn
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

/** Background operations and screen state that survives Activity recreation. */
object AppState {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)

    /** Pop-up message at the bottom of the screen. */
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

    // ---- once a second: play counting and sleep timer ----
    sealed interface Sleep { data class At(val ms: Long) : Sleep; data class EndOf(val id: String) : Sleep }
    val sleep = MutableStateFlow<Sleep?>(null)
    private var ticking = false
    private var lastId: String? = null
    private var counted: String? = null

    fun startTicker() {
        if (ticking) return
        ticking = true
        scope.launch {
            while (true) {
                kotlinx.coroutines.delay(1000)
                val st = PlayerConn.state.value
                val id = st.currentId
                if (id != lastId) {
                    val prev = lastId
                    lastId = id; counted = null
                    // "after this track": once it finishes, pause before the next one
                    if ((sleep.value as? Sleep.EndOf)?.id == prev && prev != null) { PlayerConn.pause(); sleep.value = null; say(L("Таймер сна: трек доиграл, спокойной ночи", "Sleep timer: the track finished, good night")) }
                }
                if (st.isPlaying && id != null && counted != id && PlayerConn.position() > 30_000 && Store.lib.value.tracks.containsKey(id)) { Store.countPlay(id); counted = id }
                val s = sleep.value
                if (s is Sleep.At && System.currentTimeMillis() >= s.ms) { PlayerConn.pause(); sleep.value = null; say(L("Таймер сна: спокойной ночи", "Sleep timer: good night")) }
            }
        }
    }

    fun setSleep(minutes: Int?) {
        sleep.value = when {
            minutes == null -> null
            minutes == 0 -> PlayerConn.state.value.currentId?.let { Sleep.EndOf(it) }
            else -> Sleep.At(System.currentTimeMillis() + minutes * 60_000L)
        }
    }

    // ---- track radio ----
    fun radio(id: String) {
        val t = Store.track(id) ?: return
        val vid = t.videoId ?: return say(L("Радио работает для треков с YouTube", "Radio works for YouTube tracks"))
        bg(L("Собираю радио по «${t.title}»…", "Building a radio from “${t.title}”…")) {
            val list = withContext(Dispatchers.IO) { YouTube.radio(vid, 30) }
            if (list.isEmpty()) return@bg say(L("Похожих треков не нашлось", "No similar tracks found"))
            Store.remember(list)
            val ids = list.map { it.id }
            if (PlayerConn.state.value.currentId == id) PlayerConn.playNext(ids) else PlayerConn.play(listOf(id) + ids)
            say(L("Радио: ${list.size} похожих треков в очереди", "Radio: ${list.size} similar tracks in the queue"))
        }
    }

    // ---- lyrics ----
    data class LyricsState(val id: String, val text: app.wavy.android.data.Lyrics.Text? = null, val error: String? = null)
    val lyrics = MutableStateFlow<LyricsState?>(null)
    fun loadLyrics(id: String) {
        if (lyrics.value?.id == id && lyrics.value?.error == null) return
        val t = Store.track(id) ?: return
        lyrics.value = LyricsState(id)
        scope.launch {
            val r = withContext(Dispatchers.IO) { runCatching { app.wavy.android.data.Lyrics.fetch(t) } }
            if (lyrics.value?.id == id) lyrics.value = LyricsState(id, r.getOrNull(), r.exceptionOrNull()?.let { it.message ?: L("LRCLIB недоступен", "LRCLIB is unavailable") })
        }
    }

    // ---- manual video choice ----
    /** replace — id of the track whose video is being changed; otherwise add Spotify track `sp` (to playlist `pid`); rest — the ones not found yet. */
    data class Pick(val query: String, val results: List<Track> = emptyList(), val busy: Boolean = true, val error: String? = null,
                    val replace: String? = null, val sp: Spotify.SpTrack? = null, val pid: String? = null, val rest: List<Spotify.SpTrack> = emptyList())
    data class NotFound(val pid: String?, val items: List<Spotify.SpTrack>)
    val pick = MutableStateFlow<Pick?>(null)
    val notFound = MutableStateFlow<NotFound?>(null)

    fun pickSearch(p: Pick) {
        pick.value = p.copy(busy = true, error = null)
        scope.launch {
            val r = withContext(Dispatchers.IO) { runCatching { Spotify.candidates(p.query) } }
            val cur = pick.value ?: return@launch
            pick.value = cur.copy(busy = false, results = r.getOrDefault(emptyList()),
                error = r.exceptionOrNull()?.message ?: if (r.getOrNull().isNullOrEmpty()) L("Ничего не нашлось — попробуй написать иначе", "Nothing found — try different wording") else null)
        }
    }

    fun pickForTrack(id: String) {
        val t = Store.track(id) ?: return
        val q = if (t.artist.isEmpty() || t.artist == "YouTube") t.title else "${t.artist.split(',').first().trim()} ${t.title}"
        pickSearch(Pick(q, replace = id))
    }

    fun pickForSpotify(nf: NotFound, i: Int) {
        val sp = nf.items[i]
        notFound.value = null
        pickSearch(Pick("${sp.artists.split(',').first().trim()} ${sp.title}".trim(), sp = sp, pid = nf.pid, rest = nf.items.filterIndexed { j, _ -> j != i }))
    }

    /** Cancel the choice: go back to the list of tracks not found, if there was one. */
    fun pickCancel() {
        val p = pick.value ?: return
        pick.value = null
        if (p.sp != null) notFound.value = NotFound(p.pid, listOf(p.sp) + p.rest)
    }

    fun pickChoose(c: Track) {
        val p = pick.value ?: return
        pick.value = null
        if (p.replace != null) {
            val old = Store.track(p.replace) ?: return
            val ytCover = old.cover == null || old.cover.contains("ytimg.com") || old.cover.contains("googleusercontent")
            Store.addTracks(listOf(old.copy(type = "youtube", videoId = c.videoId, url = c.url, cover = if (ytCover) c.cover else old.cover,
                duration = if (c.duration > 0) c.duration else old.duration)))
            c.videoId?.let { YouTube.invalidate(it) }
            say(L("«${old.title}»: видео заменено — со следующего запуска трека", "“${old.title}”: video replaced — takes effect next time the track plays"))
        } else if (p.sp != null) {
            val t = Spotify.fromCandidate(c, p.sp)
            Store.addTracks(listOf(t))
            p.pid?.let { Store.addToPlaylist(it, listOf(t.id)) }
            say(L("Добавлено: ${t.title}", "Added: ${t.title}"))
            if (p.rest.isNotEmpty()) notFound.value = NotFound(p.pid, p.rest)
        }
    }

    private fun bg(label: String, block: suspend () -> Unit) = scope.launch {
        busy.value = label
        try { block() } catch (e: Throwable) { say(L("Ошибка: ${e.message ?: e.javaClass.simpleName}", "Error: ${e.message ?: e.javaClass.simpleName}")) } finally { busy.value = null }
    }

    fun scanLocal(ctx: Context) = bg(L("Ищу музыку и видео на телефоне…", "Looking for music and videos on the phone…")) {
        val found = withContext(Dispatchers.IO) { LocalScanner.scan(ctx) }
        Store.addTracks(found)
        // anime episodes: videos with an episode number in the name → titles, covers and names from Shikimori
        val ids = found.filter { it.video }.map { it.id }.toSet()
        val pending = Store.addEpisodes(LocalScanner.videoNames.filterKeys { it in ids })
        say(if (found.isEmpty()) L("На телефоне не нашлось музыки", "No music found on the phone") else L("Найдено: ${found.count { !it.video }} треков, ${found.count { it.video }} видео", "Found: ${found.count { !it.video }} tracks, ${found.count { it.video }} videos"))
        if (pending.isNotEmpty()) matchAnime(pending)
    }

    /** Look up titles on Shikimori. Single-"episode" videos that aren't there aren't anime: return them to the library. */
    fun matchAnime(list: List<app.wavy.android.data.Anime>) = bg(L("Ищу аниме на Шикимори…", "Looking up anime on Shikimori…")) {
        for (a in list) {
            val p = AnimeParse.Parsed(a.title, null, if ("#s" in a.key) a.key.substringAfter("#s").toIntOrNull() else null)
            val hit = runCatching { withContext(Dispatchers.IO) { Shikimori.bestMatch(a.title, p.season, a.episodes.size, Shikimori.search(AnimeParse.searchQuery(p))) } }.getOrNull()
            if (hit == null) {
                if (a.episodes.size < 2) Store.deleteAnime(a.id) else Store.editAnime(a.id) { it.copy(matched = true) }
                continue
            }
            Store.editAnime(a.id) { it.copy(shikiId = hit.id, name = hit.name, russian = hit.russian, english = hit.english, kind = hit.kind, episodesTotal = hit.episodes, score = hit.score, year = hit.year, poster = hit.poster, url = hit.url, matched = true) }
            val title = (if (app.wavy.android.data.Lang.en) hit.english else hit.russian).ifEmpty { hit.name }
            Store.addTracks(a.episodes.mapNotNull { Store.track(it) }.map { it.copy(artist = title, cover = hit.poster ?: it.cover) })
        }
    }

    /** Watch a title: from the last unfinished episode (or the chosen one). */
    fun playAnime(id: String, episode: String? = null) {
        val a = Store.anime(id) ?: return
        if (a.episodes.isEmpty()) return
        val start = episode?.let { a.episodes.indexOf(it) }?.takeIf { it >= 0 }
            ?: a.last?.let { l -> a.episodes.indexOf(l).takeIf { it >= 0 }?.let { i -> if (l in a.watched && i + 1 < a.episodes.size) i + 1 else i } }
            ?: a.episodes.indexOfFirst { it !in a.watched }.coerceAtLeast(0)
        app.wavy.android.player.PlayerConn.play(a.episodes, start)
    }

    // ---------- Wi-Fi sync with the PC ----------
    /** Download tracks for offline listening. */
    fun downloadOffline(ids: List<String>) = bg(L("Скачиваю для офлайна…", "Downloading for offline…")) {
        val tracks = ids.mapNotNull { Store.track(it) }.filter { it.videoId != null && !app.wavy.android.data.Offline.has(it) }
        if (tracks.isEmpty()) { say(L("Всё уже скачано", "Everything is already downloaded")); return@bg }
        var ok = 0
        var failed = 0
        for ((i, t) in tracks.withIndex()) {
            busy.value = L("Скачиваю для офлайна: ${i + 1} из ${tracks.size} · ${t.title}", "Downloading for offline: ${i + 1} of ${tracks.size} · ${t.title}")
            try { withContext(Dispatchers.IO) { app.wavy.android.data.Offline.download(t) }; ok++ } catch (e: Exception) { failed++ }
        }
        say(L("Скачано для офлайна: $ok", "Downloaded for offline: $ok") + if (failed > 0) L(", не вышло: $failed", ", failed: $failed") else "")
    }

    /** Whether the PC remote is open. */
    val remoteOpen = MutableStateFlow(false)

    fun wifiSync(addr: String, pin: String) = bg(L("Синхронизация с ПК…", "Syncing with the PC…")) {
        val base = "http://" + addr.trim().removePrefix("http://").trimEnd('/').let { if (':' in it) it else "$it:3738" } + "/wavy/sync?pin=" + pin.trim()
        val r = withContext(Dispatchers.IO) {
            val fromPc = app.wavy.android.net.Http.get(base)
            val got = Sync.import(fromPc)
            val req = okhttp3.Request.Builder().url(base).post(okhttp3.RequestBody.create("application/json".toMediaTypeOrNull(), Sync.export())).build()
            app.wavy.android.net.Http.client.newCall(req).execute().use { resp -> if (!resp.isSuccessful) throw java.io.IOException(if (resp.code == 403) L("неверный PIN", "wrong PIN") else L("ПК ответил ${resp.code}", "The PC replied ${resp.code}")) }
            got
        }
        say(L("С ПК: ${r.tracks} треков, новых плейлистов ${r.newPlaylists}. Плейлисты телефона отправлены на ПК", "From the PC: ${r.tracks} tracks, new playlists ${r.newPlaylists}. The phone's playlists were sent to the PC"))
    }

    // ---------- Shazam ----------
    sealed interface Shz { data object Idle : Shz; data object Listening : Shz; data class Found(val f: Shazam.Found) : Shz; data object NotFound : Shz; data class Error(val msg: String) : Shz; data object Adding : Shz }
    val shazam = MutableStateFlow<Shz>(Shz.Idle)
    val shazamOpen = MutableStateFlow(false)
    /** The Activity asks for microphone permission and calls listen(). */
    var micRequest: (() -> Unit)? = null
    fun startShazam() { shazamOpen.value = true; micRequest?.invoke() ?: listen() }

    /** Listen to the microphone for 12 seconds and ask Shazam. The Activity checks the microphone permission. */
    @android.annotation.SuppressLint("MissingPermission")
    fun listen() = scope.launch {
        shazam.value = Shz.Listening
        shazam.value = try {
            val samples = withContext(Dispatchers.IO) { recordMic(Shazam.SECONDS) }
            withContext(Dispatchers.IO) { Shazam.recognize(samples) }?.let { Shz.Found(it) } ?: Shz.NotFound
        } catch (e: Throwable) { Shz.Error(e.message ?: L("не получилось", "didn't work")) }
    }

    @android.annotation.SuppressLint("MissingPermission")
    private fun recordMic(secs: Int): FloatArray {
        val rate = Shazam.RATE
        val min = android.media.AudioRecord.getMinBufferSize(rate, android.media.AudioFormat.CHANNEL_IN_MONO, android.media.AudioFormat.ENCODING_PCM_16BIT)
        val rec = android.media.AudioRecord(android.media.MediaRecorder.AudioSource.MIC, rate, android.media.AudioFormat.CHANNEL_IN_MONO, android.media.AudioFormat.ENCODING_PCM_16BIT, maxOf(min, rate))
        if (rec.state != android.media.AudioRecord.STATE_INITIALIZED) { rec.release(); throw IllegalStateException(L("микрофон недоступен", "the microphone is unavailable")) }
        val buf = ShortArray(rate * secs)
        rec.startRecording()
        try {
            var got = 0
            while (got < buf.size) { val n = rec.read(buf, got, buf.size - got); if (n <= 0) break; got += n }
        } finally { rec.stop(); rec.release() }
        return FloatArray(buf.size) { buf[it] / 32768f }
    }

    /** Add what was found: the track is looked up on YouTube and goes into the library and the "Openings" playlist. */
    fun addFound(f: Shazam.Found) = scope.launch {
        shazam.value = Shz.Adding
        try {
            val t = withContext(Dispatchers.IO) { YouTube.search("${f.artist} ${f.title}", music = true, limit = 1).firstOrNull() } ?: throw IllegalStateException(L("на YouTube не нашлось", "not found on YouTube"))
            Store.addTracks(listOf(t))
            val pl = Store.lib.value.playlists.firstOrNull { it.name == "Опенинги" || it.name == "Openings" }?.id ?: Store.createPlaylist(L("Опенинги", "Openings")).id
            Store.addToPlaylist(pl, listOf(t.id))
            say(L("«${f.title}» — в библиотеке и плейлисте «Опенинги»", "“${f.title}” — in the library and the playlist “Openings”"))
            shazam.value = Shz.Idle
        } catch (e: Throwable) { shazam.value = Shz.Error(e.message ?: L("не получилось добавить", "couldn't add it")) }
    }

    /** Link → tracks (Spotify / YouTube / YouTube Music / SoundCloud…). onDone(playlistId). */
    fun importUrl(url: String, intoPlaylist: String? = null, onDone: (String?) -> Unit = {}) = bg(L("Импорт…", "Importing…")) {
        val progress: (String) -> Unit = { m -> scope.launch { busy.value = m } }
        val s = Store.lib.value.settings
        var missing: List<Spotify.SpTrack> = emptyList()
        val (name, source, tracks, note) = withContext(Dispatchers.IO) {
            if (Spotify.isSpotify(url)) {
                val r = Spotify.import(url, s.spotifyClientId, s.spotifyClientSecret, progress)
                var note = ""
                if (r.notFound.isNotEmpty()) note += L(", не нашлось на YouTube: ${r.notFound.size} — можно выбрать вручную", ", not found on YouTube: ${r.notFound.size} — you can pick them manually")
                missing = r.notFound
                if (r.truncated) note += L(". Без ключа Spotify — только первые 100 треков", ". Without a key Spotify gives only the first 100 tracks")
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
        say(L("Добавлено ${tracks.size} трек(ов)$note", "Added ${tracks.size} track(s)$note"))
        onDone(pid)
        if (missing.isNotEmpty()) notFound.value = NotFound(pid, missing)
    }

    fun syncPlaylist(id: String) {
        val p = Store.lib.value.playlists.firstOrNull { it.id == id } ?: return
        val src = p.source?.takeIf { it.startsWith("http") } ?: return say(L("У плейлиста нет ссылки-источника", "The playlist has no source link"))
        bg(L("Синхронизирую «${p.name}»…", "Syncing “${p.name}”…")) {
            val s = Store.lib.value.settings
            val tracks = withContext(Dispatchers.IO) {
                if (Spotify.isSpotify(src)) Spotify.import(src, s.spotifyClientId, s.spotifyClientSecret).tracks else YouTube.importUrl(src).tracks
            }
            Store.addTracks(tracks)
            Store.setPlaylistTracks(id, tracks.map { it.id })
            say(L("Синхронизировано: ${tracks.size} треков", "Synced: ${tracks.size} tracks"))
        }
    }

    fun search() {
        val q = searchQuery.value.trim()
        if (q.isEmpty()) return
        if (q.startsWith("http")) { importUrl(q); return }
        scope.launch {
            searching.value = true
            try { searchResults.value = withContext(Dispatchers.IO) { YouTube.search(q, searchMusic.value) } }
            catch (e: Throwable) { say(L("Поиск не удался: ${e.message}", "Search failed: ${e.message}")) }
            finally { searching.value = false }
        }
    }

    fun importExport(text: String) = bg(L("Импорт плейлистов…", "Importing playlists…")) {
        val r = withContext(Dispatchers.IO) { Sync.import(text) }
        say(L("Импортировано: ${r.tracks} треков, новых плейлистов ${r.newPlaylists}, обновлено ${r.updatedPlaylists}", "Imported: ${r.tracks} tracks, new playlists ${r.newPlaylists}, updated ${r.updatedPlaylists}") + if (r.skippedLocal > 0) L(" (локальные файлы с ПК не переносятся: ${r.skippedLocal})", " (local files from the PC aren't transferred: ${r.skippedLocal})") else "")
    }

    fun checkUpdate(manual: Boolean) = scope.launch {
        update.value = Upd.Checking
        update.value = try {
            withContext(Dispatchers.IO) { Updater.check() }?.let { Upd.Available(it) } ?: Upd.UpToDate
        } catch (e: Throwable) { if (manual) Upd.Error(e.message ?: L("не удалось проверить", "couldn't check")) else Upd.Idle }
    }

    fun downloadUpdate(ctx: Context, rel: Updater.Release) = scope.launch {
        update.value = Upd.Downloading(0)
        update.value = try {
            val f = withContext(Dispatchers.IO) { Updater.download(ctx, rel) { p -> update.value = Upd.Downloading(p) } }
            Upd.Ready(f)
        } catch (e: Throwable) { Upd.Error(e.message ?: L("не удалось скачать", "couldn't download")) }
    }

    private data class Quad(val name: String?, val source: String, val tracks: List<Track>, val note: String)
}
