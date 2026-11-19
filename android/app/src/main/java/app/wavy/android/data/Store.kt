package app.wavy.android.data

import android.content.Context
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import java.io.File

/** Библиотека: треки, плейлисты, настройки. Хранится в files/library.json. */
object Store {
    private lateinit var file: File
    private val _lib = MutableStateFlow(Library())
    val lib: StateFlow<Library> = _lib
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private var saveJob: Job? = null

    fun init(ctx: Context) {
        if (::file.isInitialized) return
        file = File(ctx.filesDir, "library.json")
        runCatching { _lib.value = json.decodeFromString(Library.serializer(), file.readText()) }
    }

    /** Для тестов: хранилище во временной папке. */
    fun initForTest(dir: File) { file = File(dir, "library.json"); _lib.value = Library() }

    private fun save() {
        saveJob?.cancel()
        saveJob = scope.launch {
            delay(300)
            flush()
        }
    }

    fun flush() {
        if (!::file.isInitialized) return
        val tmp = File(file.parentFile, "library.json.tmp")
        tmp.writeText(json.encodeToString(Library.serializer(), _lib.value))
        tmp.renameTo(file)
    }

    private fun edit(f: (Library) -> Library) { _lib.update(f); save() }

    /** Треки из поиска, которые ещё не в библиотеке (живут до закрытия приложения). */
    private val temp = java.util.concurrent.ConcurrentHashMap<String, Track>()
    fun remember(list: List<Track>) { list.forEach { temp[it.id] = it } }

    fun track(id: String) = _lib.value.tracks[id] ?: temp[id]

    fun addTracks(list: List<Track>) = edit { l ->
        val m = l.tracks.toMutableMap()
        for (t in list) m[t.id] = m[t.id]?.let { old -> t.copy(addedAt = old.addedAt) } ?: t
        l.copy(tracks = m)
    }

    fun removeTracks(ids: Set<String>) = edit { l ->
        l.copy(tracks = l.tracks - ids, playlists = l.playlists.map { p -> p.copy(trackIds = p.trackIds.filterNot { it in ids }) })
    }

    fun createPlaylist(name: String, ids: List<String> = emptyList(), source: String? = null): Playlist {
        val now = System.currentTimeMillis()
        val p = Playlist("pl_${now.toString(36)}${(1000..9999).random()}", name, ids.distinct(), source, now)
        edit { it.copy(playlists = it.playlists + p) }
        return p
    }

    private fun editPlaylist(id: String, f: (Playlist) -> Playlist) = edit { l -> l.copy(playlists = l.playlists.map { if (it.id == id) f(it) else it }) }
    fun renamePlaylist(id: String, name: String) = editPlaylist(id) { it.copy(name = name) }
    fun setPlaylistTracks(id: String, ids: List<String>) = editPlaylist(id) { it.copy(trackIds = ids.distinct()) }
    fun addToPlaylist(id: String, ids: List<String>) = editPlaylist(id) { p -> p.copy(trackIds = (p.trackIds + ids).distinct()) }
    fun removeFromPlaylist(id: String, ids: Set<String>) = editPlaylist(id) { p -> p.copy(trackIds = p.trackIds.filterNot { it in ids }) }
    fun deletePlaylist(id: String) = edit { l -> l.copy(playlists = l.playlists.filterNot { it.id == id }) }

    fun settings(f: (Settings) -> Settings) = edit { it.copy(settings = f(it.settings)) }
    fun session(s: Session) = edit { it.copy(session = s) }

    /** Треки библиотеки, новые сверху. */
    fun libraryIds(l: Library = _lib.value) = l.tracks.values.sortedWith(compareByDescending<Track> { it.addedAt }.thenBy { it.title }).map { it.id }
}
