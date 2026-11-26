package app.wavy.android

import app.wavy.android.data.AnimeParse
import app.wavy.android.data.Lyrics
import app.wavy.android.data.Track
import app.wavy.android.data.json
import app.wavy.android.yt.Spotify
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.double
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * Cases shared with the PC version (files in testdata/parity, also read by core/tests/parity.rs).
 * The algorithms are duplicated in Kotlin and Rust — these tests catch one side drifting from the other.
 * The name contains "LogicTest" so CI runs them with the rest.
 */
class ParityLogicTest {
    private fun cases(name: String): List<JsonObject> {
        // Gradle runs tests from android/app; search upward for testdata
        var dir: File? = File("").absoluteFile
        while (dir != null && !File(dir, "testdata/parity").isDirectory) dir = dir.parentFile
        val f = File(dir ?: error("не нашёл testdata/parity"), "testdata/parity/$name.json")
        return json.parseToJsonElement(f.readText()).jsonObject["cases"]!!.jsonArray.map { it.jsonObject }
    }

    private fun JsonObject.s(k: String) = this[k]!!.jsonPrimitive.content
    private fun JsonElement?.numOrNull() = (this as? JsonPrimitive)?.doubleOrNull

    @Test fun animeNames() {
        val bad = cases("anime").mapNotNull { c ->
            val p = AnimeParse.parseName(c.s("name"))
            val got = listOf(p.title, p.episode, p.season, AnimeParse.key(p), AnimeParse.searchQuery(p))
            val want = listOf(c.s("title"), c["episode"].numOrNull(), (c["season"] as? JsonPrimitive)?.intOrNull, c.s("key"), c.s("query"))
            if (got != want) "${c.s("name")}\n  ждали $want\n  вышло $got" else null
        }
        assertTrue(bad.joinToString("\n"), bad.isEmpty())
    }

    @Test fun spotifyCleanTitle() {
        for (c in cases("spotify_clean")) assertEquals(c.s("in"), c.s("out"), Spotify.cleanTitle(c.s("in")))
    }

    @Test fun spotifyMatch() {
        for (c in cases("spotify_match")) {
            val sp = c["spotify"]!!.jsonObject
            val t = Spotify.SpTrack(sp.s("title"), sp.s("artists"), sp.s("album"), sp["duration"]!!.jsonPrimitive.double)
            val cands = c["candidates"]!!.jsonArray.map { json.decodeFromJsonElement(Track.serializer(), it) }
            val best = Spotify.match(t) { cands }?.videoId
            assertEquals(c.s("what"), (c["expect"] as? JsonPrimitive)?.contentOrNull, best)
        }
    }

    @Test fun lyricsClean() {
        for (c in cases("lyrics_clean")) {
            val t = Track("x", c.s("type"), title = c.s("title"), artist = c.s("artist"))
            val want = c["clean"]!!.jsonArray.let { it[0].jsonPrimitive.content to it[1].jsonPrimitive.content }
            assertEquals(c.s("title"), want, Lyrics.clean(t))
        }
    }

    @Test fun lyricsLrc() {
        for (c in cases("lyrics_lrc")) {
            val want = (c["lines"] as JsonArray).map { l -> l.jsonArray.let { it[0].jsonPrimitive.double to it[1].jsonPrimitive.content } }
            assertEquals(c.s("lrc"), want, Lyrics.parseLrc(c.s("lrc")))
        }
    }
}
