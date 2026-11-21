package app.wavy.android

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onRoot
import app.wavy.android.data.Store
import app.wavy.android.data.Track
import app.wavy.android.player.PlayerConn
import app.wavy.android.ui.AppState
import app.wavy.android.ui.AnimeScreen
import app.wavy.android.ui.ShazamDialog
import app.wavy.android.data.Shazam
import app.wavy.android.ui.Bg
import app.wavy.android.ui.FullPlayer
import app.wavy.android.ui.LibraryScreen
import app.wavy.android.ui.MiniPlayer
import app.wavy.android.ui.PlaylistsScreen
import app.wavy.android.ui.SearchScreen
import app.wavy.android.ui.SettingsScreen
import app.wavy.android.ui.WavyTheme
import com.github.takahirom.roborazzi.captureRoboImage
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import java.nio.file.Files

/** Скриншоты экранов без телефона — PNG в build/outputs/roborazzi */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w400dp-h860dp-xxhdpi")
class ScreenshotTest {
    @get:Rule val rule = createComposeRule()

    private fun t(i: Int, title: String, artist: String, dur: Double, yt: Boolean = true) =
        Track(if (yt) "yt_vid0000000$i" else "and_a_$i", if (yt) "youtube" else "local", videoId = if (yt) "vid0000000$i" else null,
            title = title, artist = artist, album = "Synth Nights", duration = dur, addedAt = 1000L - i)

    @Before fun data() {
        Store.initForTest(Files.createTempDirectory("shot").toFile())
        val tracks = listOf(
            t(1, "Midnight Drive", "Neon Waves", 214.0, yt = false), t(2, "Nightcall", "Kavinsky", 257.0),
            t(3, "Get Lucky (feat. Pharrell Williams)", "Daft Punk", 249.0), t(4, "Feel This Moment", "Pitbull, Christina Aguilera", 229.0),
            t(5, "Snowman", "Lofi Girl", 196.0), t(6, "Stardancer", "Virtua", 185.0),
        )
        Store.addTracks(tracks)
        Store.createPlaylist("Synthwave 🌌", tracks.drop(1).map { it.id }, "https://music.youtube.com/playlist?list=PL1")
        Store.createPlaylist("Для зала", tracks.take(3).map { it.id })
        PlayerConn.fakeState(PlayerConn.State(currentId = "yt_vid00000002", isPlaying = true, queue = tracks.map { it.id }, index = 1))
        AppState.searchResults.value = tracks.drop(2)
        AppState.searchQuery.value = "daft punk"
    }

    private fun shot(name: String, content: @Composable () -> Unit) {
        rule.setContent { WavyTheme { Box(Modifier.fillMaxSize().background(Bg)) { content() } } }
        rule.waitForIdle()
        rule.onRoot().captureRoboImage("build/outputs/roborazzi/$name.png")
    }

    @Test fun library() = shot("1-library") { Column { Box(Modifier.weight(1f)) { LibraryScreen({}, {}, {}) }; MiniPlayer {} } }
    @Test fun playlists() = shot("2-playlists") { PlaylistsScreen(null, {}, {}, {}) }
    @Test fun playlistDetail() = shot("3-playlist") { PlaylistsScreen(Store.lib.value.playlists.first().id, {}, {}, {}) }
    @Test fun search() = shot("4-search") { SearchScreen {} }
    @Test fun player() = shot("5-player") { FullPlayer {} }
    @Test fun settings() = shot("6-settings") { SettingsScreen({}, {}, {}, {}) }

    private fun animeData() {
        val eps = (1..4).map { Track("and_v_$it", "local", path = "content://x/$it", title = "[SubsPlease] Sousou no Frieren - 0$it (1080p)", duration = 1440.0, video = true) }
        Store.addTracks(eps)
        Store.addEpisodes(eps.associate { it.id to (it.title to "Frieren") })
        val a = Store.lib.value.anime.first()
        Store.editAnime(a.id) { it.copy(russian = "Провожающая в последний путь Фрирен", name = "Sousou no Frieren", kind = "tv", year = 2023, episodesTotal = 28, score = 9.3, matched = true) }
        Store.markWatched("and_v_1")
    }

    @Test fun anime() { animeData(); shot("7-anime") { AnimeScreen(null, {}, {}, {}, {}) } }
    @Test fun animeTitle() { animeData(); shot("8-anime-title") { AnimeScreen(Store.lib.value.anime.first().id, {}, {}, {}, {}) } }
    @Test fun shazam() {
        AppState.shazam.value = AppState.Shz.Found(Shazam.Found("Yuusha", "YOASOBI", null, null))
        shot("9-shazam") { ShazamDialog {} }
    }
}
