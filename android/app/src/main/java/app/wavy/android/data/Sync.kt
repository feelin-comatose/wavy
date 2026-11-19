package app.wavy.android.data

import kotlinx.serialization.Serializable

/**
 * Обмен плейлистами между ПК и телефоном: файл wavy-export.json (одинаковый формат в обеих версиях).
 * Переносятся YouTube/Spotify/стрим-треки и плейлисты; локальные файлы — нет (их пути у каждого устройства свои).
 */
object Sync {
    @Serializable data class ExportPlaylist(val name: String, val source: String? = null, val tracks: List<Track>)
    @Serializable data class Export(val wavyExport: Int = 1, val app: String = "android", val exportedAt: Long = 0, val playlists: List<ExportPlaylist> = emptyList(), val tracks: List<Track> = emptyList())
    data class Summary(val tracks: Int, val newPlaylists: Int, val updatedPlaylists: Int, val skippedLocal: Int)

    fun export(lib: Library = Store.lib.value): String {
        val portable = lib.tracks.values.filter { !it.isLocal }
        val pls = lib.playlists.map { p -> ExportPlaylist(p.name, p.source, p.trackIds.mapNotNull { lib.tracks[it] }.filter { !it.isLocal }) }.filter { it.tracks.isNotEmpty() }
        return json.encodeToString(Export.serializer(), Export(exportedAt = System.currentTimeMillis(), playlists = pls, tracks = portable))
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
        return Summary(portable.size, created, updated, all.count { it.isLocal })
    }
}
