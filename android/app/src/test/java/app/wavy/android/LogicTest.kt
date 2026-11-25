package app.wavy.android

import app.wavy.android.data.AnimeParse
import app.wavy.android.data.Shazam
import app.wavy.android.data.Shikimori
import app.wavy.android.data.Store
import app.wavy.android.data.Sync
import app.wavy.android.data.Track
import app.wavy.android.yt.Spotify
import org.junit.Assert.assertArrayEquals
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

    private fun p(s: String) = AnimeParse.parseName(s).let { Triple(it.title, it.episode, it.season) }

    /** Те же случаи, что в тестах версии для ПК (core/src/anime.rs). */
    @Test fun animeFansubNames() {
        assertEquals(Triple("Sousou no Frieren", 1.0, null), p("[SubsPlease] Sousou no Frieren - 01 (1080p) [ABCD1234]"))
        assertEquals(Triple("Kimetsu no Yaiba - Hashira Geiko-hen", 8.0, null), p("[Erai-raws] Kimetsu no Yaiba - Hashira Geiko-hen - 08 [1080p][Multiple Subtitle][F2A7C0D1]"))
        assertEquals(Triple("Jujutsu Kaisen", 12.0, null), p("[AniLibria.TV] Jujutsu Kaisen - 12 [WEBRip 1080p]"))
        assertEquals(Triple("Jujutsu Kaisen", 5.0, null), p("Jujutsu_Kaisen_-_05_[AniLibria.TV]_[WEBRip_1080p]"))
        assertEquals(Triple("Jujutsu Kaisen", 1.0, null), p("Jujutsu_Kaisen_[01]_[AniLibria_TV]_[WEBRip_1080p_HEVC]"))
        assertEquals(Triple("Chainsaw Man", 5.0, null), p("[AniLibria.TV] Chainsaw Man [05 из 12] [1080p]"))
        assertEquals(Triple("Shingeki no Kyojin", 16.0, 4), p("[Judas] Shingeki no Kyojin - S04E16v2 [1080p][HEVC x265 10bit]"))
        assertEquals(Triple("Spy x Family", 3.0, 2), p("Spy.x.Family.S02E03.1080p.WEB-DL.AAC.x264"))
        assertEquals(Triple("Mushoku Tensei S2", 13.0, 2), p("[SubsPlease] Mushoku Tensei S2 - 13 (1080p) [0B3A1B7C]"))
        assertEquals(Triple("One Piece", 1100.0, null), p("[SubsPlease] One Piece - 1100 (1080p) [ABCDEF12]"))
        assertEquals(Triple("Oshi no Ko", 11.5, null), p("[Group] Oshi no Ko - 11.5 (1080p)"))
        assertEquals(Triple("Chainsaw Man", 3.0, null), p("Chainsaw Man Episode 3"))
        assertEquals(Triple("Chainsaw Man", 3.0, null), p("Chainsaw Man 03"))
        assertEquals(Triple("Атака титанов", 7.0, null), p("Атака титанов - Серия 7"))
        assertEquals(Triple("Атака титанов", 7.0, null), p("07. Атака титанов"))
        assertEquals(Triple("Kimi no Na wa", null, null), p("Kimi no Na wa (2016) [BDRip 1080p]"))
        assertEquals("Dr. Stone", p("Dr. Stone - 05 [1080p]").first)
        // название из папки, если в имени файла только номер
        assertEquals("Тетрадь смерти", AnimeParse.parse("05", "Тетрадь смерти").title)
        assertEquals(AnimeParse.key(AnimeParse.parseName("[A] Mushoku Tensei S2 - 01")), AnimeParse.key(AnimeParse.parseName("[B] Mushoku Tensei Season 2 - 02")))
        assertEquals("Mushoku Tensei 2", AnimeParse.searchQuery(AnimeParse.parseName("[SubsPlease] Mushoku Tensei S2 - 13 (1080p)")))
    }

    @Test fun shikimoriPicksRightSeason() {
        fun a(id: Long, name: String, ru: String, kind: String, eps: Int) = Shikimori.Hit(id, name, ru, "", "", emptyList(), kind, eps, 0.0, 0, null, "")
        val list = listOf(a(1, "Sousou no Frieren", "Провожающая в последний путь Фрирен", "tv", 28), a(2, "Sousou no Frieren: ●● no Mahou", "", "ona", 24), a(3, "Sousou no Frieren 2nd Season", "Провожающая в последний путь Фрирен 2", "tv", 10))
        assertEquals(1L, Shikimori.bestMatch("Sousou no Frieren", null, 12, list)?.id)
        assertEquals(3L, Shikimori.bestMatch("Sousou no Frieren", 2, 10, list)?.id)
        assertNull(Shikimori.bestMatch("Совсем другое", null, 3, list))
    }

    /** Отпечаток Shazam совпадает с версией для ПК (core/src/shazam.rs, тест vector::dump). */
    @Test fun shazamMatchesDesktop() {
        var x = 12345
        val sig = FloatArray(Shazam.RATE * 4) { i ->
            x = x * 1103515245 + 12345
            val noise = ((x ushr 16) and 0x7ff) - 1024
            val period = 16 + (i / 4000) % 40
            val sq = if ((i / period) % 2 == 0) 6000 else -6000
            (sq + noise) / 32768f
        }
        val ours = Shazam.signature(sig)
        val hex = javaClass.classLoader!!.getResource("shazam_desktop.hex")!!.readText().trim()
        val desktop = ByteArray(hex.length / 2) { hex.substring(it * 2, it * 2 + 2).toInt(16).toByte() }
        assertEquals(desktop.size, ours.size)
        val diff = desktop.indices.count { desktop[it] != ours[it] }
        // заголовок (кроме CRC) — один в один; в пиках допускаем редкие расхождения в последнем бите округления
        assertTrue("различий: $diff", diff < desktop.size / 50 + 8)
        assertEquals(desktop.copyOfRange(8, 48).toList(), ours.copyOfRange(8, 48).toList())
    }

    @Test fun offlineChunks() {
        // сервер с Range, как googlevideo: файл 10 000 байт, качаем кусками по 3000
        val data = ByteArray(10_000) { (it % 251).toByte() }
        val srv = java.net.ServerSocket(0, 50, java.net.InetAddress.getByName("127.0.0.1"))
        val th = Thread {
            while (!srv.isClosed) {
                val c = try { srv.accept() } catch (e: Exception) { break }
                c.use {
                    val head = c.getInputStream().bufferedReader().let { r -> generateSequence { r.readLine() }.takeWhile { it.isNotEmpty() }.toList() }
                    val (a, b) = head.first { it.startsWith("Range:", true) }.substringAfter("bytes=").split('-').map { it.trim().toInt() }
                    val end = minOf(b, data.size - 1)
                    val o = c.getOutputStream()
                    o.write("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes $a-$end/${data.size}\r\nContent-Length: ${end - a + 1}\r\nConnection: close\r\n\r\n".toByteArray())
                    o.write(data, a, end - a + 1)
                    o.flush()
                }
            }
        }.apply { isDaemon = true; start() }
        try {
            val f = java.io.File.createTempFile("wavy", ".part")
            app.wavy.android.data.Offline.fetch("http://127.0.0.1:${srv.localPort}/a", f, chunk = 3000)
            assertArrayEquals(data, f.readBytes())
        } finally { srv.close(); th.join(1000) }
    }

    @Test fun likesSyncAndSmartLists() {
        Store.initForTest(Files.createTempDirectory("likes").toFile())
        Store.addTracks(listOf(yt("aaaaaaaaaaa", "A", "X", 100.0), yt("bbbbbbbbbbb", "B", "X", 100.0), yt("ccccccccccc", "C", "X", 100.0)))
        Store.setLiked(listOf("yt_aaaaaaaaaaa"), true)
        Store.setLiked(listOf("yt_bbbbbbbbbbb"), true)
        assertEquals(listOf("yt_bbbbbbbbbbb", "yt_aaaaaaaaaaa"), Store.lib.value.liked)
        val now = 100L * 86_400_000L
        Store.countPlay("yt_aaaaaaaaaaa", now - 1000); Store.countPlay("yt_aaaaaaaaaaa", now - 2000); Store.countPlay("yt_ccccccccccc", now - 40L * 86_400_000L)
        assertEquals(listOf("yt_aaaaaaaaaaa"), Store.smart("week", now = now))
        assertEquals(listOf("yt_aaaaaaaaaaa", "yt_ccccccccccc"), Store.smart("alltime", now = now))
        assertEquals(listOf("yt_bbbbbbbbbbb"), Store.smart("forgotten", now = now))
        assertEquals(listOf("yt_bbbbbbbbbbb"), Store.smart("never", now = now))
        val text = Sync.export()
        Store.initForTest(Files.createTempDirectory("likes2").toFile())
        Sync.import(text)
        assertEquals(listOf("yt_bbbbbbbbbbb", "yt_aaaaaaaaaaa"), Store.lib.value.liked)
        // лайки с ПК: неизвестный трек пропускается
        Sync.import("""{"wavyExport":1,"app":"desktop","liked":["yt_ccccccccccc","yt_gone"],"tracks":[]}""")
        assertTrue(Store.isLiked("yt_ccccccccccc") && !Store.isLiked("yt_gone"))
        Store.removeTracks(setOf("yt_ccccccccccc"))
        assertTrue(!Store.isLiked("yt_ccccccccccc"))
    }

    @Test fun lyricsParseAndClean() {
        val l = app.wavy.android.data.Lyrics.parseLrc("[00:01.50] one\n[00:10.00][01:00.00] chorus\nplain")
        assertEquals(listOf(1.5 to "one", 10.0 to "chorus", 60.0 to "chorus"), l)
        val t = app.wavy.android.data.Lyrics.Text(l)
        assertEquals(-1, t.lineAt(0.5)); assertEquals(1, t.lineAt(30.0))
        assertEquals("YOASOBI" to "アイドル", app.wavy.android.data.Lyrics.clean(yt("x", "YOASOBI「アイドル」Official Music Video", "Ayase", 200.0)))
        assertEquals("LiSA" to "Gurenge", app.wavy.android.data.Lyrics.clean(yt("y", "LiSA - Gurenge (Official Video)", "LiSA Official", 200.0)))
    }

    @Test fun spotifyCleanTitle() {
        assertEquals("come together", Spotify.cleanTitle("Come Together - Remastered 2009"))
        assertEquals("stay", Spotify.cleanTitle("Stay (feat. Justin Bieber)"))
        assertEquals("song (live)", Spotify.cleanTitle("Song (Live)"))
    }
}
