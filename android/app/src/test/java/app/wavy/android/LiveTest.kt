package app.wavy.android

import app.wavy.android.yt.Spotify
import app.wavy.android.yt.YouTube
import org.junit.Assume
import org.junit.Before
import org.junit.Test

/** Живые проверки с сетью: ./gradlew testDebugUnitTest -Plive=true --tests '*LiveTest*' */
class LiveTest {
    @Before fun live() = Assume.assumeTrue(System.getProperty("live") == "true")

    @Test fun musicSearch() {
        val r = YouTube.search("daft punk get lucky", music = true, limit = 5)
        r.forEach { println("LIVE search: ${it.artist} — ${it.title} [${it.duration}] ${it.id}") }
        check(r.isNotEmpty())
    }

    @Test fun ytMusicPlaylist() {
        val r = YouTube.importUrl("https://music.youtube.com/playlist?list=PLY1Ew7TDxB6U") { }
        println("LIVE playlist: «${r.name}» ${r.tracks.size} треков; первый: ${r.tracks.firstOrNull()?.title}")
        check(r.tracks.size > 100)
    }

    @Test fun streamUrls() {
        println("LIVE audio: " + YouTube.audioStream("dQw4w9WgXcQ").take(60))
        println("LIVE video: " + YouTube.videoStream("dQw4w9WgXcQ").take(60))
    }

    @Test fun spotifyAlbum() {
        val t = System.currentTimeMillis()
        val r = Spotify.import("https://open.spotify.com/album/4aawyAB9vmqN3uQ7FjRGTy")
        println("LIVE spotify: «${r.name}» найдено ${r.tracks.size}, не найдено ${r.notFound.size}, ${(System.currentTimeMillis() - t) / 1000} с")
        r.tracks.take(4).forEach { println("LIVE   ${it.artist} — ${it.title} → ${it.videoId}") }
        check(r.tracks.size >= 15)
    }
}
