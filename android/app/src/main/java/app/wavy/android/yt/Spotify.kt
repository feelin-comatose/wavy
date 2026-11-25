package app.wavy.android.yt

import app.wavy.android.data.L
import app.wavy.android.data.Track
import app.wavy.android.net.Http
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.contentOrNull
import okhttp3.FormBody
import okhttp3.Request
import java.util.Base64
import java.util.concurrent.atomic.AtomicInteger

/** Импорт из Spotify (как на ПК): список треков → поиск каждого на YouTube Music. */
object Spotify {
    data class SpTrack(val title: String, val artists: String, val album: String, val duration: Double)
    class SpList(val name: String, val tracks: List<SpTrack>, val maybeTruncated: Boolean)
    class Result(val name: String, val source: String, val tracks: List<Track>, val notFound: List<SpTrack>, val truncated: Boolean, val kind: String)

    private val js = Json { ignoreUnknownKeys = true }

    /** open.spotify.com/(intl-xx/)playlist|album|track/ID или spotify:playlist:ID */
    fun parseUrl(url: String): Pair<String, String>? {
        val u = url.trim()
        if (u.startsWith("spotify:")) {
            val p = u.removePrefix("spotify:").split(':')
            return if (p.size >= 2) p[0] to p[1] else null
        }
        val m = Regex("""https?://[^/]*spotify\.com/(.*)""").find(u) ?: return null
        val segs = m.groupValues[1].substringBefore('?').split('/').filter { it.isNotEmpty() && !it.startsWith("intl-") && it != "embed" }
        return if (segs.size >= 2 && segs[0] in setOf("playlist", "album", "track")) segs[0] to segs[1] else null
    }

    fun isSpotify(url: String) = parseUrl(url) != null

    private operator fun JsonElement?.get(k: String): JsonElement? = (this as? JsonObject)?.get(k)
    private val JsonElement?.str get() = (this?.takeIf { it !is JsonNull })?.jsonPrimitive?.contentOrNull
    private val JsonElement?.num get() = (this?.takeIf { it !is JsonNull })?.jsonPrimitive?.doubleOrNull

    /** Публичная embed-страница: без ключей, до 100 треков. */
    fun fetchEmbed(kind: String, id: String): SpList {
        val html = Http.get("https://open.spotify.com/embed/$kind/$id", mapOf("Accept-Language" to "en"))
        val start = html.indexOf("""<script id="__NEXT_DATA__" type="application/json">""")
        require(start >= 0) { L("Spotify изменил страницу — не нашёл данные плейлиста", "Spotify changed the page — couldn't find the playlist data") }
        val body = html.substring(html.indexOf('>', start) + 1).substringBefore("</script>")
        val e = js.parseToJsonElement(body)["props"]["pageProps"]["state"]["data"]["entity"]
        require(e != null && e !is JsonNull) { L("плейлист не найден или приватный", "the playlist wasn't found or is private") }
        val name = e["name"].str ?: "Spotify"
        val album = if (kind == "album") name else ""
        val tracks = if (kind == "track") listOf(SpTrack(name, e["artists"]?.jsonArray?.joinToString(", ") { it["name"].str ?: "" } ?: "", "", (e["duration"].num ?: 0.0) / 1000))
        else e["trackList"]?.jsonArray?.mapNotNull { t ->
            val title = t["title"].str ?: return@mapNotNull null
            SpTrack(title, (t["subtitle"].str ?: "").replace(' ', ' '), album, (t["duration"].num ?: 0.0) / 1000)
        } ?: emptyList()
        return SpList(name, tracks, kind == "playlist" && tracks.size >= 100)
    }

    /** Официальный Web API: весь плейлист любого размера (нужны Client ID и Secret). */
    fun fetchApi(kind: String, id: String, clientId: String, secret: String): SpList {
        val auth = Base64.getEncoder().encodeToString("$clientId:$secret".toByteArray())
        val tokReq = Request.Builder().url("https://accounts.spotify.com/api/token")
            .header("Authorization", "Basic $auth").post(FormBody.Builder().add("grant_type", "client_credentials").build()).build()
        val token = Http.client.newCall(tokReq).execute().use { r ->
            require(r.isSuccessful) { L("Spotify API: не удалось войти (проверь Client ID / Secret)", "Spotify API: couldn't sign in (check the Client ID / Secret)") }
            js.parseToJsonElement(r.body!!.string())["access_token"].str ?: error(L("Spotify API не выдал токен", "Spotify API didn't give a token"))
        }
        fun api(url: String) = js.parseToJsonElement(Http.get(url, mapOf("Authorization" to "Bearer $token")))
        fun artists(t: JsonElement?) = t["artists"]?.jsonArray?.joinToString(", ") { it["name"].str ?: "" } ?: ""
        fun mk(t: JsonElement?, album: String?) = SpTrack(t["name"].str ?: "", artists(t), album ?: t["album"]["name"].str ?: "", (t["duration_ms"].num ?: 0.0) / 1000)
        if (kind == "track") { val t = api("https://api.spotify.com/v1/tracks/$id"); return SpList(t["name"].str ?: "Spotify", listOf(mk(t, null)), false) }
        val name = api("https://api.spotify.com/v1/${kind}s/$id?fields=name")["name"].str ?: "Spotify"
        val out = ArrayList<SpTrack>()
        var next: String? = "https://api.spotify.com/v1/${kind}s/$id/tracks?limit=${if (kind == "album") 50 else 100}"
        while (next != null) {
            val page = api(next)
            for (it in page["items"]?.jsonArray ?: emptyList()) {
                val t = if (kind == "playlist") it["track"] else it
                if (t == null || t is JsonNull || t["type"].str == "episode") continue
                out += mk(t, if (kind == "album") name else null)
            }
            next = page["next"].str
        }
        return SpList(name, out, false)
    }

    /** Название без хвостов, которых нет у видео: «(feat. X)», «- Remastered 2011», «[Explicit]» (как на ПК). */
    fun cleanTitle(s: String): String {
        var t = s.lowercase()
        for ((open, close) in listOf('(' to ')', '[' to ']')) {
            while (true) {
                val a = t.indexOf(open).takeIf { it >= 0 } ?: break
                val b = t.indexOf(close, a).takeIf { it >= 0 } ?: break
                val inner = t.substring(a + 1, b)
                if (listOf("feat", "ft.", "with ", "remaster", "radio edit", "from ", "bonus", "explicit").any { inner.contains(it) }) t = t.removeRange(a, b + 1) else break
            }
        }
        val dash = t.indexOf(" - ")
        if (dash >= 0 && listOf("remaster", "radio edit", "feat", "mono", "stereo", "single version", "album version").any { t.substring(dash + 3).contains(it) }) t = t.substring(0, dash)
        return t.split(Regex("\\s+")).filter { it.isNotEmpty() }.joinToString(" ")
    }

    /** Кандидаты: песни YouTube Music, а если их мало — ещё и видео YouTube. Сбои сети повторяются. */
    fun candidates(q: String): List<Track> {
        fun retry(block: () -> List<Track>): List<Track> {
            repeat(3) { i -> runCatching { return block() }; Thread.sleep(800L * (i + 1)) }
            return emptyList()
        }
        val out = ArrayList(retry { YouTube.search(q, music = true, limit = 8) })
        if (out.size < 3) for (c in retry { YouTube.search(q, music = false, limit = 6) }) if (out.none { it.videoId == c.videoId }) out += c
        return out
    }

    /** Лучший кандидат: название, исполнитель, длительность, «официальность» (та же оценка, что на ПК). */
    fun match(t: SpTrack, search: (String) -> List<Track> = ::candidates): Track? {
        val mainArtist = t.artists.split(',').first().trim()
        val cands = search(if (mainArtist.isEmpty()) t.title else "$mainArtist ${t.title}")
        val title = cleanTitle(t.title)
        val artist = mainArtist.lowercase()
        val best = cands.mapIndexed { i, c ->
            var score = 0.0
            val ct = c.title.lowercase()
            val ctc = cleanTitle(c.title)
            val titleOk = ct.contains(title) || ctc == title || (ctc.length > 3 && title.contains(ctc))
            val artistOk = artist.isNotEmpty() && (c.artist.lowercase().contains(artist) || ct.contains(artist))
            if (titleOk) score += 8
            if (artistOk) score += 5
            if (t.duration > 0 && c.duration > 0) {
                val d = kotlin.math.abs(c.duration - t.duration)
                score -= d.coerceAtMost(120.0) / (if (titleOk && artistOk) 10 else 4)
                if (d > 30 && !(titleOk && artistOk)) score -= 20
            }
            for (bad in listOf("live", "cover", "remix", "karaoke", "8d", "slowed", "sped up", "nightcore", "reaction", "instrumental"))
                if (ct.contains(bad) && !title.contains(bad)) score -= 8
            if (c.url?.contains("music.youtube") == true && c.album.isNotEmpty()) score += 3
            if (ct.contains("audio")) score += 2
            score -= i * 0.3
            score to c
        }.maxByOrNull { it.first }
        return best?.takeIf { it.first > -25 }?.second?.let { fromCandidate(it, t) }
    }

    /** Трек библиотеки из найденного видео: название, исполнитель и альбом — из Spotify. */
    fun fromCandidate(c: Track, t: SpTrack) = c.copy(
        url = "https://music.youtube.com/watch?v=${c.videoId}", title = t.title.ifEmpty { c.title },
        artist = t.artists.ifEmpty { c.artist }, album = t.album.ifEmpty { c.album },
        duration = if (t.duration > 0) t.duration else c.duration,
    )

    fun import(url: String, clientId: String = "", secret: String = "", progress: (String) -> Unit = {}): Result {
        val (kind, id) = parseUrl(url) ?: throw IllegalArgumentException(L("это не ссылка на Spotify", "this isn't a Spotify link"))
        progress(L("Spotify: читаю список…", "Spotify: reading the list…"))
        val list = if (clientId.isNotBlank() && secret.isNotBlank()) fetchApi(kind, id, clientId, secret) else fetchEmbed(kind, id)
        require(list.tracks.isNotEmpty()) { L("в плейлисте Spotify нет треков (или он приватный)", "the Spotify playlist has no tracks (or it's private)") }
        val n = list.tracks.size
        val done = AtomicInteger()
        val sem = Semaphore(6)
        val found = runBlocking(Dispatchers.IO) {
            list.tracks.map { t ->
                async { sem.withPermit { match(t).also { val d = done.incrementAndGet(); if (d % 5 == 0 || d == n) progress(L("Spotify: ищу на YouTube $d/$n…", "Spotify: searching YouTube $d/$n…")) } } }
            }.awaitAll()
        }
        val base = System.currentTimeMillis()
        val tracks = ArrayList<Track>()
        val notFound = ArrayList<SpTrack>()
        val seen = HashSet<String>()
        found.forEachIndexed { i, t ->
            when {
                t == null -> notFound += list.tracks[i]
                seen.add(t.id) -> tracks += t.copy(addedAt = base + (n - i))
            }
        }
        return Result(list.name, url.trim(), tracks, notFound, list.maybeTruncated, kind)
    }
}
