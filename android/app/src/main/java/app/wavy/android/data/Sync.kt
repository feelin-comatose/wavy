package app.wavy.android.data

import kotlinx.serialization.Serializable

/**
 * Playlist exchange between PC and phone: the wavy-export.json file (same format in both versions).
 * YouTube/Spotify/stream tracks and playlists are transferred; local files are not (each device has its own paths).
 */
object Sync {
    @Serializable data class ExportPlaylist(val name: String, val source: String? = null, val tracks: List<Track>)
    @Serializable data class Export(val wavyExport: Int = 1, val app: String = "android", val exportedAt: Long = 0, val playlists: List<ExportPlaylist> = emptyList(), val tracks: List<Track> = emptyList(),
        /** likes (ids of transferable tracks), newest first; merged on import */
        val liked: List<String> = emptyList())
    data class Summary(val tracks: Int, val newPlaylists: Int, val updatedPlaylists: Int, val skippedLocal: Int)

    fun export(lib: Library = Store.lib.value): String {
        val portable = lib.tracks.values.filter { !it.isLocal }
        val pls = lib.playlists.map { p -> ExportPlaylist(p.name, p.source, p.trackIds.mapNotNull { lib.tracks[it] }.filter { !it.isLocal }) }.filter { it.tracks.isNotEmpty() }
        val liked = lib.liked.filter { id -> lib.tracks[id]?.isLocal == false }
        return json.encodeToString(Export.serializer(), Export(exportedAt = System.currentTimeMillis(), playlists = pls, tracks = portable, liked = liked))
    }

    fun isExport(text: String) = text.trimStart().startsWith("{") && text.contains("\"wavyExport\"")

    fun import(text: String): Summary {
        val e = json.decodeFromString(Export.serializer(), text)
        val all = (e.tracks + e.playlists.flatMap { it.tracks })
        val portable = all.filter { !it.isLocal }.distinctBy { it.id }
        Store.addTracks(portable)
        var created = 0; var updated = 0
        for (p in e.playlists) {
            val ids = p.tracks.filter { !it.isLocal }.map { it.id }
            val existing = Store.lib.value.playlists.firstOrNull { (p.source != null && it.source == p.source) || it.name == p.name }
            if (existing != null) { Store.addToPlaylist(existing.id, ids); updated++ } else { Store.createPlaylist(p.name, ids, p.source); created++ }
        }
        val have = Store.lib.value
        val newLikes = e.liked.filter { it in have.tracks && it !in have.liked }
        if (newLikes.isNotEmpty()) Store.setLiked(newLikes, true)
        return Summary(portable.size, created, updated, all.count { it.isLocal })
    }
}
