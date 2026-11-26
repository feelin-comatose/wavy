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

/** Library: tracks, playlists, settings. Stored in files/library.json. */
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
        Lang.set(_lib.value.settings.lang)
        app.wavy.android.ui.Themes.set(_lib.value.settings.theme, _lib.value.settings.accent)
    }

    /** For tests: storage in a temporary folder. */
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

    /** Tracks from search that aren't in the library yet (live until the app closes). */
    private val temp = java.util.concurrent.ConcurrentHashMap<String, Track>()
    fun remember(list: List<Track>) { list.forEach { temp[it.id] = it } }

    fun track(id: String) = _lib.value.tracks[id] ?: temp[id]

    fun addTracks(list: List<Track>) = edit { l ->
        val m = l.tracks.toMutableMap()
        for (t in list) m[t.id] = m[t.id]?.let { old -> t.copy(addedAt = old.addedAt) } ?: t
        l.copy(tracks = m)
    }

    fun removeTracks(ids: Set<String>) = edit { l ->
        l.copy(tracks = l.tracks - ids, playlists = l.playlists.map { p -> p.copy(trackIds = p.trackIds.filterNot { it in ids }) }, liked = l.liked.filterNot { it in ids })
    }

    // ---- likes and plays ----
    fun isLiked(id: String, l: Library = _lib.value) = id in l.liked
    fun setLiked(ids: List<String>, on: Boolean) = edit { l ->
        val rest = l.liked.filterNot { it in ids }
        l.copy(liked = if (on) ids + rest else rest)
    }
    /** The track played for longer than 30 seconds. */
    fun countPlay(id: String, now: Long = System.currentTimeMillis()) = edit { l ->
        l.copy(plays = l.plays + (id to (l.plays[id] ?: 0) + 1), history = (l.history + Play(id, now)).takeLast(20_000))
    }

    /** Smart playlists: (rule, name, description) — same as on the PC. */
    fun smartRules() = listOf(
        Triple("week", L("Топ недели", "Top of the week"), L("Чаще всего за 7 дней", "Most played in 7 days")),
        Triple("month", L("Топ месяца", "Top of the month"), L("Чаще всего за 30 дней", "Most played in 30 days")),
        Triple("alltime", L("На повторе", "On repeat"), L("Самое слушаемое за всё время", "Most played of all time")),
        Triple("forgotten", L("Забытое любимое", "Forgotten favorites"), L("Лайкнутое, что не играло больше месяца", "Liked, but not played for over a month")),
        Triple("fresh", L("Свежее", "Fresh"), L("Добавлено за две недели, ещё не слушал", "Added in the last two weeks, not played yet")),
        Triple("never", L("Ни разу не слушал", "Never played"), L("Всё, что ещё ни разу не играло", "Everything that has never been played")),
    )

    /** Tracks of a smart playlist (without anime episodes). */
    fun smart(rule: String, l: Library = _lib.value, now: Long = System.currentTimeMillis()): List<String> {
        val day = 86_400_000L
        val music = libraryIds(l)
        val inLib = music.toHashSet()
        val last = HashMap<String, Long>()
        for (p in l.history) last[p.id] = maxOf(last[p.id] ?: 0L, p.at)
        fun top(since: Long) = l.history.filter { it.at >= since && it.id in inLib }.groupingBy { it.id }.eachCount()
            .entries.sortedByDescending { it.value }.take(50).map { it.key }
        return when (rule) {
            "week" -> top(now - 7 * day)
            "month" -> top(now - 30 * day)
            "alltime" -> l.plays.filter { it.value > 0 && it.key in inLib }.entries.sortedByDescending { it.value }.take(50).map { it.key }
            "forgotten" -> l.liked.filter { it in inLib && (last[it] ?: 0L) < now - 30 * day }
            "fresh" -> music.filter { (l.tracks[it]?.addedAt ?: 0L) >= now - 14 * day && (l.plays[it] ?: 0) == 0 }
            "never" -> music.filter { (l.plays[it] ?: 0) == 0 }
            else -> emptyList()
        }
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

    fun settings(f: (Settings) -> Settings) { edit { it.copy(settings = f(it.settings)) }; Lang.set(_lib.value.settings.lang); app.wavy.android.ui.Themes.set(_lib.value.settings.theme, _lib.value.settings.accent) }
    fun session(s: Session) = edit { it.copy(session = s) }

    /** Library tracks, newest first (anime episodes live in their own section). */
    fun libraryIds(l: Library = _lib.value): List<String> {
        val episodes = l.anime.flatMapTo(HashSet()) { it.episodes }
        return l.tracks.values.filter { it.id !in episodes }.sortedWith(compareByDescending<Track> { it.addedAt }.thenBy { it.title }).map { it.id }
    }

    fun anime(id: String) = _lib.value.anime.firstOrNull { it.id == id }
    fun editAnime(id: String, f: (Anime) -> Anime) = edit { l -> l.copy(anime = l.anime.map { if (it.id == id) f(it) else it }) }
    fun deleteAnime(id: String) = edit { l -> l.copy(anime = l.anime.filterNot { it.id == id }) }

    /**
     * Phone videos → anime titles: file name and folder are parsed as on the PC, episodes are grouped by title.
     * `names` — track id → (file name without extension, folder). Returns titles not yet looked up on Shikimori.
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
            // episodes by number
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
