package app.wavy.android

import app.wavy.android.data.Store
import app.wavy.android.data.Sync
import app.wavy.android.data.Track
import app.wavy.android.yt.Spotify
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.nio.file.Files

class LogicTest {
    @Test fun spotifyUrls() {
        assertEquals("playlist" to "37i9dQZF1DXcBWIGoYBM5M", Spotify.parseUrl("https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M?si=abc"))
        assertEquals("album" to "4aawyAB9vmqN3uQ7FjRGTy", Spotify.parseUrl("https://open.spotify.com/intl-ru/album/4aawyAB9vmqN3uQ7FjRGTy"))
        assertEquals("track" to "3h5T5JypYU7huFiVYhv1dr", Spotify.parseUrl("spotify:track:3h5T5JypYU7huFiVYhv1dr"))
        assertNull(Spotify.parseUrl("https://music.youtube.com/playlist?list=x"))
    }

    private fun yt(id: String, title: String, artist: String, dur: Double) = Track("yt_$id", "youtube", videoId = id, url = "https://www.youtube.com/watch?v=$id", title = title, artist = artist, duration = dur)

    @Test fun spotifyMatchPrefersRightVersion() {
        val sp = Spotify.SpTrack("Feel This Moment (feat. Christina Aguilera)", "Pitbull, Christina Aguilera", "Global Warming", 229.0)
        val cands = listOf(
            yt("aaaaaaaaaaa", "Feel This Moment (Live)", "Pitbull", 260.0),
            yt("bbbbbbbbbbb", "Feel This Moment (feat. Christina Aguilera)", "Pitbull", 230.0),
            yt("ccccccccccc", "Feel This Moment (Remix)", "DJ Someone", 200.0),
        )
        val m = Spotify.match(sp) { cands }
        assertNotNull(m)
        assertEquals("yt_bbbbbbbbbbb", m!!.id)
        assertEquals("Pitbull, Christina Aguilera", m.artist) // имя из Spotify
        assertTrue(m.url!!.startsWith("https://music.youtube.com/"))
    }

    @Test fun spotifyMatchRejectsGarbage() {
        val sp = Spotify.SpTrack("Song", "Artist", "", 200.0)
        assertNull(Spotify.match(sp) { listOf(yt("xxxxxxxxxxx", "Totally different 10h mix", "Other", 36000.0)) })
    }

    @Test fun syncRoundTripAndDesktopFormat() {
        Store.initForTest(Files.createTempDirectory("wavy").toFile())
        Store.addTracks(listOf(yt("aaaaaaaaaaa", "A", "X", 100.0), Track("and_a_1", "local", path = "content://x", title = "Local")))
        Store.createPlaylist("Mix", listOf("yt_aaaaaaaaaaa", "and_a_1"), "https://music.youtube.com/playlist?list=PL1")
        val text = Sync.export()
        assertTrue(Sync.isExport(text))
        assertTrue("локальные треки не уходят", !text.contains("content://x"))
        // на «другом устройстве»
        Store.initForTest(Files.createTempDirectory("wavy2").toFile())
        val r = Sync.import(text)
        assertEquals(1, r.tracks); assertEquals(1, r.newPlaylists)
        assertEquals(listOf("yt_aaaaaaaaaaa"), Store.lib.value.playlists.single().trackIds)
        // повторный импорт не плодит дубликаты
        val r2 = Sync.import(text)
        assertEquals(0, r2.newPlaylists); assertEquals(1, Store.lib.value.playlists.size)
        // файл с ПК: лишние поля игнорируются
        val desktop = """{"wavyExport":1,"app":"desktop","playlists":[{"name":"Desk","tracks":[{"id":"yt_bbbbbbbbbbb","type":"youtube","videoId":"bbbbbbbbbbb","title":"B","artist":"Y","duration":5.0,"addedAt":1,"futureField":true}]}],"tracks":[]}"""
        Sync.import(desktop)
        assertNotNull(Store.track("yt_bbbbbbbbbbb"))
    }
}
