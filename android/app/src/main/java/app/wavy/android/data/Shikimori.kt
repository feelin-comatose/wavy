package app.wavy.android.data

import app.wavy.android.net.Http
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody

/** Шикимори: поиск тайтла (GraphQL, без ключа) — как в версии для ПК. */
object Shikimori {
    private val HOSTS = listOf("https://shikimori.io", "https://shikimori.one")
    private const val UA = "Wavy (Android music & anime player)"

    data class Hit(
        val id: Long, val name: String, val russian: String, val english: String, val japanese: String, val synonyms: List<String>,
        val kind: String, val episodes: Int, val score: Double, val year: Int, val poster: String?, val url: String,
    )

    fun search(q: String, limit: Int = 8): List<Hit> {
        val query = "query(\$search: String, \$limit: PositiveInt) { animes(search: \$search, limit: \$limit) { id name russian english japanese synonyms kind episodes score airedOn { year } poster { mainUrl originalUrl } url } }"
        val body = buildJsonObject { put("query", query); put("variables", buildJsonObject { put("search", q); put("limit", limit) }) }.toString()
        var last: Exception? = null
        for (host in HOSTS) {
            try {
                val req = Request.Builder().url("$host/api/graphql").header("User-Agent", UA).post(body.toRequestBody("application/json".toMediaType())).build()
                Http.client.newCall(req).execute().use { r ->
                    val v = json.parseToJsonElement(r.body?.string() ?: "").jsonObject
                    val list = (v["data"] as? JsonObject)?.get("animes") as? JsonArray ?: return emptyList()
                    return list.map { parse(it.jsonObject, host) }
                }
            } catch (e: Exception) { last = e }
        }
        throw last ?: java.io.IOException(L("Шикимори недоступен", "Shikimori is unavailable"))
    }

    private fun parse(a: JsonObject, host: String): Hit {
        fun s(k: String) = (a[k] as? JsonPrimitive)?.contentOrNull ?: ""
        val poster = (a["poster"] as? JsonObject)?.let { p -> (p["originalUrl"] as? JsonPrimitive)?.contentOrNull ?: (p["mainUrl"] as? JsonPrimitive)?.contentOrNull }?.takeIf { "missing" !in it }
        val url = s("url").let { if (it.startsWith("http") || it.isEmpty()) it else host + it }
        return Hit(
            s("id").toLongOrNull() ?: 0, s("name"), s("russian"), s("english"), s("japanese"),
            (a["synonyms"] as? JsonArray)?.mapNotNull { (it as? JsonPrimitive)?.contentOrNull } ?: emptyList(),
            s("kind"), (a["episodes"] as? JsonPrimitive)?.intOrNull ?: 0, (a["score"] as? JsonPrimitive)?.doubleOrNull ?: 0.0,
            ((a["airedOn"] as? JsonObject)?.get("year") as? JsonPrimitive)?.intOrNull ?: 0, poster, url,
        )
    }

    private fun norm(s: String) = s.lowercase().filter { it.isLetterOrDigit() }

    /** Похожесть строк: доля общих биграмм (Дайс). */
    fun similarity(a0: String, b0: String): Double {
        val a = norm(a0); val b = norm(b0)
        if (a.isEmpty() || b.isEmpty()) return 0.0
        if (a == b) return 1.0
        val x = a.windowed(2); val y = b.windowed(2).toMutableList()
        if (x.isEmpty() || y.isEmpty()) return 0.0
        val total = x.size + y.size
        var common = 0
        for (p in x) { val i = y.indexOf(p); if (i >= 0) { y.removeAt(i); common++ } }
        return 2.0 * common / total
    }

    /** Выбрать тайтл: похожесть названия, сезон, число серий (как на ПК). */
    fun bestMatch(title: String, season: Int?, episodes: Int, list: List<Hit>): Hit? {
        val want = season ?: 1
        return list.map { a ->
            var score = (listOf(a.name, a.russian, a.english, a.japanese) + a.synonyms).maxOf { similarity(title, it) }
            fun has(n: Int): Boolean {
                val l = a.name.lowercase()
                val suf = when (n) { 2 -> "nd"; 3 -> "rd"; else -> "th" }
                return "season $n" in l || "$n$suf season" in l || l.endsWith(" $n") || a.russian.endsWith(" $n") || "$n сезон" in a.russian
            }
            if (want > 1) score += if (has(want)) 0.3 else -0.15 else if ((2..6).any(::has)) score -= 0.2
            when {
                episodes == 1 && a.kind == "movie" -> score += 0.1
                episodes > 1 && a.kind == "tv" -> score += 0.1
                episodes > 2 && a.kind == "movie" -> score -= 0.3
            }
            if (a.kind in setOf("cm", "pv", "music")) score -= 0.35
            if (a.episodes > 0 && episodes > a.episodes + 2) score -= 0.1
            score to a
        }.filter { it.first > 0.35 }.maxByOrNull { it.first }?.second
    }
}
