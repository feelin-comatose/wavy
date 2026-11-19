package app.wavy.android.ui

import android.content.Context
import app.wavy.android.data.LocalScanner
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
        say(if (found.isEmpty()) "На телефоне не нашлось музыки" else "Найдено: ${found.count { !it.video }} треков, ${found.count { it.video }} видео")
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
