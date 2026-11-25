package app.wavy.android.data

import app.wavy.android.net.Http
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.doubleOrNull
import java.io.File
import java.net.URLEncoder
import java.security.MessageDigest

/**
 * Тексты песен с LRCLIB (открытая база, без ключа): синхронные (построчно со временем) или простые.
 * Ответы кэшируются на диске — второй раз без интернета. Тот же разбор, что на ПК (core/src/lyrics.rs).
 */
object Lyrics {
    data class Text(val synced: List<Pair<Double, String>> = emptyList(), val plain: String = "", val instrumental: Boolean = false) {
        val isEmpty get() = synced.isEmpty() && plain.isBlank() && !instrumental
        /** Номер текущей строки (секунды → индекс), -1 — ещё не началась. */
        fun lineAt(t: Double): Int = synced.indexOfLast { it.first <= t + 0.15 }
    }

    private var dir: File? = null
    fun init(filesDir: File) { dir = File(filesDir, "lyrics").apply { mkdirs() } }

    /** «[01:23.45] текст» → (83.45, «текст»). Несколько меток на строке — несколько строк. */
    fun parseLrc(s: String): List<Pair<Double, String>> {
        val out = ArrayList<Pair<Double, String>>()
        for (line in s.lines()) {
            var rest = line.trim()
            val times = ArrayList<Double>()
            while (rest.startsWith("[")) {
                val end = rest.indexOf(']').takeIf { it > 0 } ?: break
                val tag = rest.substring(1, end)
                val (m, sec) = tag.split(':', limit = 2).takeIf { it.size == 2 } ?: break
                val t = (m.trim().toDoubleOrNull() ?: break) * 60 + (sec.trim().toDoubleOrNull() ?: break)
                times += t
                rest = rest.substring(end + 1).trimStart()
            }
            for (t in times) out += t to rest.trim()
        }
        return out.sortedBy { it.first }
    }

    private val junk = Regex("""(?i)\s*[(\[【][^)\]】]*(official|video|audio|lyric|mv|m/v|hd|4k|remaster|visuali[sz]er|music|clip|full|ver\.?|version|opening|ending)[^)\]】]*[)\]】]""")
    private val tail = Regex("""(?i)\s*(official\s+)?(music\s+)?(video|audio|mv|lyric video)\s*$""")
    private val jp = Regex("""^(.*?)\s*「(.+?)」""")
    private val feat = Regex("""(?i)\s+(feat\.?|ft\.?)\s.*$""")

    /** Название без мусора YouTube: «(Official Video)», «[MV]», «feat.»; «Артист - Песня» → разделить. */
    fun clean(t: Track): Pair<String, String> {
        var artist = t.artist.removeSuffix(" - Topic").trim()
        var title = tail.replace(junk.replace(t.title, ""), "")
        val m = jp.find(title)
        if (m != null) {
            if (m.groupValues[1].isNotBlank()) artist = m.groupValues[1].trim()
            title = m.groupValues[2]
        } else {
            val sep = listOf(" - ", " – ", " — ").firstOrNull { title.contains(it) }
            if (sep != null && (!t.isLocal || artist.isEmpty())) {
                artist = title.substringBefore(sep).trim()
                title = title.substringAfter(sep).trim()
            }
        }
        return artist to feat.replace(title, "").trim().trim('"').trim()
    }

    private fun str(e: JsonElement?) = (e as? JsonPrimitive)?.contentOrNull
    private fun fromJson(v: JsonElement?): Text {
        val o = v as? JsonObject ?: return Text()
        return Text(str(o["syncedLyrics"])?.let(::parseLrc) ?: emptyList(), str(o["plainLyrics"]) ?: "", (o["instrumental"] as? JsonPrimitive)?.booleanOrNull ?: false)
    }

    private fun getJson(url: String): JsonElement? = try {
        json.parseToJsonElement(Http.get(url, mapOf("User-Agent" to "Wavy (https://github.com/feelin-comatose/wavy)")))
    } catch (e: java.io.IOException) {
        if (e.message?.contains("404") == true) null else throw e
    }

    private fun key(id: String) = MessageDigest.getInstance("SHA-1").digest(id.toByteArray()).joinToString("") { "%02x".format(it) }.take(16)

    /** Найти текст трека (сначала в кэше). Блокирующая — вызывай не из главного потока. */
    fun fetch(t: Track): Text {
        val file = dir?.let { File(it, key(t.id) + ".json") }
        file?.takeIf { it.exists() }?.let { f -> runCatching { return fromJson(json.parseToJsonElement(f.readText())) } }
        val (artist, title) = clean(t)
        if (title.isEmpty()) return Text()
        fun enc(s: String) = URLEncoder.encode(s, "UTF-8")
        var found: JsonElement? = null
        if (artist.isNotEmpty()) {
            var u = "https://lrclib.net/api/get?artist_name=${enc(artist)}&track_name=${enc(title)}"
            if (t.duration > 0) u += "&duration=${Math.round(t.duration)}"
            found = getJson(u)
        }
        if (found == null || found is JsonNull) {
            val list = getJson("https://lrclib.net/api/search?q=${enc("$artist $title".trim())}") as? JsonArray ?: JsonArray(emptyList())
            fun close(v: JsonElement): Boolean { val d = ((v as? JsonObject)?.get("duration") as? JsonPrimitive)?.doubleOrNull ?: 0.0; return t.duration <= 0 || kotlin.math.abs(d - t.duration) < 8 }
            found = list.filter(::close).firstOrNull { str((it as? JsonObject)?.get("syncedLyrics")) != null } ?: list.firstOrNull(::close)
        }
        val v = found ?: JsonNull
        runCatching { file?.writeText(v.toString()) }
        return fromJson(v)
    }
}
