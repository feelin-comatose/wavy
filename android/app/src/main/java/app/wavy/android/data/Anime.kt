package app.wavy.android.data

import kotlinx.serialization.Serializable

/** Тайтл аниме: серии — видео с телефона, название и постер — с Шикимори (как в версии для ПК). */
@Serializable
data class Anime(
    val id: String,
    /** нормализованное название из имён файлов — по нему серии собираются в тайтл */
    val key: String,
    val title: String,
    val shikiId: Long? = null,
    val name: String = "",
    val russian: String = "",
    val kind: String = "",
    val episodesTotal: Int = 0,
    val score: Double = 0.0,
    val year: Int = 0,
    val poster: String? = null,
    val url: String? = null,
    /** серии (id треков) по порядку */
    val episodes: List<String> = emptyList(),
    val watched: List<String> = emptyList(),
    val last: String? = null,
    val matched: Boolean = false,
    val addedAt: Long = 0,
) {
    val display get() = russian.ifEmpty { name.ifEmpty { title } }
    val meta get() = listOfNotNull(
        kindRu(kind).takeIf { it.isNotEmpty() }, year.takeIf { it > 0 }?.toString(),
        episodesTotal.takeIf { it > 0 }?.let { "$it эп." }, score.takeIf { it > 0 }?.let { "★ %.2f".format(it) },
    ).joinToString(" · ")

    companion object {
        fun kindRu(k: String) = when (k) {
            "tv" -> "ТВ"; "movie" -> "Фильм"; "ova" -> "OVA"; "ona" -> "ONA"; "special", "tv_special" -> "Спешл"; "music" -> "Клип"; "pv", "cm" -> "Промо"; else -> k
        }
    }
}

/** Разбор имён файлов серий — тот же алгоритм, что в Wavy для ПК (core/src/anime.rs). */
object AnimeParse {
    data class Parsed(val title: String, val episode: Double?, val season: Int?)

    private val JUNK = setOf(
        "1080p", "720p", "480p", "2160p", "4k", "1080", "720", "x264", "x265", "h264", "h265", "h.264", "h.265", "hevc", "avc", "aac", "flac", "ac3", "opus",
        "10bit", "10-bit", "8bit", "hi10p", "web", "web-dl", "webdl", "webrip", "bd", "bdrip", "bdremux", "bluray", "blu-ray", "dvd", "dvdrip", "tvrip", "hdtv",
        "rus", "eng", "jap", "jpn", "sub", "subs", "dub", "multi", "multisub", "raw", "uncensored", "remux", "dual", "audio", "hdr", "sdr",
    )

    private fun isJunk(word: String): Boolean {
        val w = word.trim { !it.isLetterOrDigit() }.lowercase()
        return w.isNotEmpty() && (w in JUNK || w.endsWith("bit"))
    }

    private val NUM = Regex("""\[(\d{1,4}(?:\.\d)?)(?:v\d)?(?:\s*(?:из|of|/)\s*\d+)?\]""")
    private val BR = Regex("""\[[^\]]*\]|【[^】]*】|\{[^}]*\}""")
    private val PAR = Regex("""\(([^)]*)\)""")
    private val DOT = Regex("""\.(\D)""")

    private fun clean(src: String): String {
        var s = NUM.replace(src) { " - ${it.groupValues[1]} " }
        s = BR.replace(s, " ")
        s = PAR.replace(s) { m ->
            val inner = m.groupValues[1]
            val crc = inner.length == 8 && inner.all { it.isDigit() || it.lowercaseChar() in 'a'..'f' }
            val year = inner.length == 4 && inner.all { it.isDigit() }
            if (crc || year || inner.split(Regex("\\s+")).any(::isJunk)) " " else m.value
        }
        if ('_' in s) s = s.replace('_', ' ')
        if (' ' !in s || s.count { it == '.' } >= 2) {
            s = DOT.replace(s) { " " + it.groupValues[1] }
            s = s.replace('.', ' ').replace("  ", " ")
        }
        return s.split(Regex("\\s+")).filter { it.isNotEmpty() }.joinToString(" ")
    }

    private fun stripTail(s: String): String {
        val w = s.split(Regex("\\s+")).filter { it.isNotEmpty() }.toMutableList()
        while (w.isNotEmpty() && (isJunk(w.last()) || w.last() in setOf("-", "–", "|"))) w.removeAt(w.size - 1)
        while (w.isNotEmpty() && w.first() in setOf("-", "–")) w.removeAt(0)
        return w.joinToString(" ")
    }

    // \b в Java не видит кириллицу — граница слова явно: не буква и не цифра рядом
    private const val B = "(?<![\\p{L}\\p{N}])"
    private const val E = "(?![\\p{L}\\p{N}])"
    private val SEASON = Regex("(?i)${B}(?:season|[Сс]езон|СЕЗОН)\\s*(\\d{1,2})$E|$B(\\d{1,2})(?:st|nd|rd|th)\\s+season$E|${B}s(\\d{1,2})$E")
    fun seasonOf(t: String): Int? = SEASON.find(t)?.groupValues?.drop(1)?.firstOrNull { it.isNotEmpty() }?.toIntOrNull()
    private val STRIP_SEASON = Regex("(?i)${B}(?:season|[Сс]езон|СЕЗОН)\\s*\\d{1,2}$E|$B\\d{1,2}(?:st|nd|rd|th)\\s+season$E|${B}s\\d{1,2}$E")
    private fun stripSeason(t: String) = STRIP_SEASON.replace(t, "")

    private val SE = Regex("(?i)^(.*?)[\\s\\-]*${B}S(\\d{1,2})\\s?E(\\d{1,4}(?:\\.\\d)?)(?:v\\d)?$E")
    private val DASH = Regex("""^(.*?)\s+[-–]\s+(?:(?i:ep|episode|e|[Сс][Ее][Рр][Ии][Яя])\.?\s*)?(\d{1,4}(?:\.\d)?)(?:v\d)?(?:\s|$)""")
    private val WORD = Regex("(?i)^(.*?)\\s*$B(?:episode|ep|e|[Сс]ерия|СЕРИЯ|[Ээ]пизод)\\.?\\s*(\\d{1,4}(?:\\.\\d)?)$E")
    private val LEAD = Regex("""^(\d{1,4})(?:\s*[.\-–)]\s*|\s+|$)(.*)$""")
    private val TAIL = Regex("""^(.*\D)\s(\d{1,4})(?:v\d)?$""")

    fun parseName(stem: String): Parsed {
        val s = clean(stem)
        SE.find(s)?.let { c -> return Parsed(stripTail(c.groupValues[1]), c.groupValues[3].toDoubleOrNull(), c.groupValues[2].toIntOrNull()) }
        DASH.find(s)?.let { c ->
            val t = stripTail(c.groupValues[1])
            if (t.isNotEmpty()) return Parsed(t, c.groupValues[2].toDoubleOrNull(), seasonOf(t))
        }
        WORD.find(s)?.let { c -> return Parsed(stripTail(c.groupValues[1]), c.groupValues[2].toDoubleOrNull(), null) }
        LEAD.find(s)?.let { c ->
            val rest = stripTail(c.groupValues[2])
            if (c.groupValues[1].length <= 3 || rest.isEmpty()) return Parsed(rest, c.groupValues[1].toDoubleOrNull(), null)
        }
        val st = stripTail(s)
        TAIL.find(st)?.let { c ->
            val t = stripTail(c.groupValues[1])
            if (t.any { it.isLetter() } && seasonOf("${c.groupValues[1]} ${c.groupValues[2]}") == null) return Parsed(t, c.groupValues[2].toDoubleOrNull(), seasonOf(t))
        }
        return Parsed(st, null, seasonOf(st))
    }

    private fun isGeneric(t: String): Boolean {
        val l = t.lowercase().trim()
        return l.isEmpty() || l in setOf("episode", "серия", "ep", "e", "эпизод", "video", "видео", "anime", "аниме", "downloads", "download", "загрузки", "сериалы", "movies", "camera")
            || l.startsWith("season") || l.startsWith("сезон")
    }

    /** Имя файла (без расширения) + папка. Если в имени нет названия («01.mkv») — берём папку. */
    fun parse(stem: String, folder: String?): Parsed {
        var p = parseName(stem)
        if ((p.title.count { it.isLetter() } < 2 || isGeneric(p.title)) && folder != null) {
            val f = parseName(folder)
            if (!isGeneric(f.title) && f.title.count { it.isLetter() } >= 2) p = p.copy(title = f.title, season = p.season ?: f.season)
            else if (p.season == null) p = p.copy(season = seasonOf(folder))
        }
        if (p.season == null) p = p.copy(season = seasonOf(p.title))
        return p
    }

    fun key(p: Parsed): String {
        val base = stripSeason(p.title).lowercase().filter { it.isLetterOrDigit() }
        return if ((p.season ?: 1) > 1) "$base#s${p.season}" else base
    }

    fun searchQuery(p: Parsed): String {
        val base = stripSeason(p.title).split(Regex("\\s+")).filter { it.isNotEmpty() }.joinToString(" ")
        return if ((p.season ?: 1) > 1) "$base ${p.season}" else base
    }

    fun episodeTitle(p: Parsed, stem: String) = p.episode?.let { e -> if (e % 1.0 == 0.0) "Серия ${e.toInt()}" else "Серия $e" } ?: stem
}
