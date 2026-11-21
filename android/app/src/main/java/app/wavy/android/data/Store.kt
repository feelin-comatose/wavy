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

    /** Треки библиотеки, новые сверху (серии аниме живут в своём разделе). */
    fun libraryIds(l: Library = _lib.value): List<String> {
        val episodes = l.anime.flatMapTo(HashSet()) { it.episodes }
        return l.tracks.values.filter { it.id !in episodes }.sortedWith(compareByDescending<Track> { it.addedAt }.thenBy { it.title }).map { it.id }
    }

    fun anime(id: String) = _lib.value.anime.firstOrNull { it.id == id }
    fun editAnime(id: String, f: (Anime) -> Anime) = edit { l -> l.copy(anime = l.anime.map { if (it.id == id) f(it) else it }) }
    fun deleteAnime(id: String) = edit { l -> l.copy(anime = l.anime.filterNot { it.id == id }) }

    /**
     * Видео с телефона → тайтлы аниме: имя файла и папка разбираются как на ПК, серии собираются по названию.
     * `names` — id трека → (имя файла без расширения, папка). Возвращает тайтлы, которые ещё не искали на Шикимори.
     */
    fun addEpisodes(names: Map<String, Pair<String, String?>>): List<Anime> {
        val now = System.currentTimeMillis()
        val parsed = names.mapValues { (_, v) -> AnimeParse.parse(v.first, v.second) }.filter { it.value.episode != null }
        if (parsed.isEmpty()) return emptyList()
        edit { l ->
            val list = l.anime.toMutableList()
            val tracks = l.tracks.toMutableMap()
            for ((tid, p) in parsed) {
                val key = AnimeParse.key(p)
                if (key.isEmpty()) continue
                val i = list.indexOfFirst { it.key == key }.takeIf { it >= 0 } ?: run { list += Anime("an_${key.hashCode().toUInt().toString(36)}", key, p.title, addedAt = now); list.size - 1 }
                val a = list[i]
                if (tid !in a.episodes) list[i] = a.copy(episodes = a.episodes + tid)
                tracks[tid]?.let { t -> tracks[tid] = t.copy(title = AnimeParse.episodeTitle(p, t.title), artist = a.display, album = "") }
            }
            // серии по номеру
            val num = parsed.mapValues { it.value.episode ?: 0.0 }
            l.copy(anime = list.map { a -> a.copy(episodes = a.episodes.sortedBy { num[it] ?: Double.MAX_VALUE }) }, tracks = tracks)
        }
        return _lib.value.anime.filter { !it.matched }
    }

    fun markWatched(trackId: String, on: Boolean = true) = edit { l ->
        l.copy(anime = l.anime.map { a ->
            if (trackId !in a.episodes) a
            else a.copy(watched = if (on) (a.watched + trackId).distinct() else a.watched - trackId, last = trackId)
        })
    }
}
