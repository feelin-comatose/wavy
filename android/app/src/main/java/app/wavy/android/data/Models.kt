package app.wavy.android.data

import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

/** A track. Fields and names match the desktop version — for playlist sync. */
@Serializable
data class Track(
    val id: String,
    val type: String, // local | youtube | stream
    val path: String? = null, // for local: content:// URI
    val videoId: String? = null,
    val url: String? = null,
    val title: String,
    val artist: String = "",
    val album: String = "",
    val duration: Double = 0.0, // seconds
    val cover: String? = null,
    val addedAt: Long = 0,
    val video: Boolean = false,
) {
    val isLocal get() = type == "local"
    val hasVideo get() = video || videoId != null
    /** Small cover for lists (saves memory and traffic). */
    val thumb get() = videoId?.let { "https://i.ytimg.com/vi/$it/mqdefault.jpg" } ?: cover
}

@Serializable
data class Playlist(
    val id: String,
    val name: String,
    val trackIds: List<String> = emptyList(),
    val source: String? = null,
    val createdAt: Long = 0,
) {
    val isRemote get() = source?.startsWith("http") == true
}

@Serializable
data class Settings(
    val shuffle: Boolean = false,
    val repeat: String = "off", // off | all | one
    val autoUpdate: Boolean = true,
    val floatingPlayer: Boolean = false,
    val spotifyClientId: String = "",
    val spotifyClientSecret: String = "",
    /** Wi-Fi sync with the PC: address (192.168.1.5:3738) and PIN from Wavy's settings on the PC */
    val syncAddr: String = "",
    val syncPin: String = "",
    /** interface language: "ru" or "en" */
    val lang: String = "ru",
    /** interface theme id (ui/Theme.kt): dark, amoled, light, ocean, sakura, forest, amber */
    val theme: String = "dark",
)

/** Saved queue — to pick up where you left off after a restart. */
@Serializable
data class Session(val queue: List<String> = emptyList(), val index: Int = 0, val positionMs: Long = 0)

@Serializable
data class Library(
    val tracks: Map<String, Track> = emptyMap(),
    val playlists: List<Playlist> = emptyList(),
    val settings: Settings = Settings(),
    val session: Session = Session(),
    val anime: List<Anime> = emptyList(),
    /** likes, newest first */
    val liked: List<String> = emptyList(),
    /** how many times the track played for longer than 30 seconds */
    val plays: Map<String, Int> = emptyMap(),
    /** what played when — for smart playlists */
    val history: List<Play> = emptyList(),
)

/** One play: track and time (ms). */
@Serializable
data class Play(val id: String, val at: Long)

val json = Json {
    ignoreUnknownKeys = true
    encodeDefaults = true
    explicitNulls = false
}

fun fmtTime(sec: Double): String {
    if (sec.isNaN() || sec <= 0) return "0:00"
    val s = sec.toLong()
    val h = s / 3600; val m = (s % 3600) / 60; val ss = s % 60
    return if (h > 0) "%d:%02d:%02d".format(h, m, ss) else "%d:%02d".format(m, ss)
}
