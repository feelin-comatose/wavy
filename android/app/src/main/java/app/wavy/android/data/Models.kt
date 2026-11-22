package app.wavy.android.data

import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

/** Трек. Поля и имена совпадают с десктопной версией — для синхронизации плейлистов. */
@Serializable
data class Track(
    val id: String,
    val type: String, // local | youtube | stream
    val path: String? = null, // для local: content:// URI
    val videoId: String? = null,
    val url: String? = null,
    val title: String,
    val artist: String = "",
    val album: String = "",
    val duration: Double = 0.0, // секунды
    val cover: String? = null,
    val addedAt: Long = 0,
    val video: Boolean = false,
) {
    val isLocal get() = type == "local"
    val hasVideo get() = video || videoId != null
    /** Маленькая обложка для списков (экономит память и трафик). */
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
    /** синхронизация с ПК по Wi-Fi: адрес (192.168.1.5:3738) и PIN из настроек Wavy на ПК */
    val syncAddr: String = "",
    val syncPin: String = "",
    /** язык интерфейса: "ru" или "en" */
    val lang: String = "ru",
)

/** Сохранённая очередь — чтобы продолжить с того же места после перезапуска. */
@Serializable
data class Session(val queue: List<String> = emptyList(), val index: Int = 0, val positionMs: Long = 0)

@Serializable
data class Library(
    val tracks: Map<String, Track> = emptyMap(),
    val playlists: List<Playlist> = emptyList(),
    val settings: Settings = Settings(),
    val session: Session = Session(),
    val anime: List<Anime> = emptyList(),
)

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
