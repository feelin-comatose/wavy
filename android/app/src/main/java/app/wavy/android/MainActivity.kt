package app.wavy.android

import app.wavy.android.data.L
import android.Manifest
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.QueueMusic
import androidx.compose.material.icons.rounded.LibraryMusic
import androidx.compose.material.icons.rounded.LiveTv
import androidx.compose.ui.unit.sp
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material.icons.rounded.Settings
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.NavigationBarItemDefaults
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.core.content.FileProvider
import app.wavy.android.data.Store
import app.wavy.android.data.Sync
import app.wavy.android.player.PlayerConn
import app.wavy.android.ui.AddToPlaylistDialog
import app.wavy.android.ui.AppState
import app.wavy.android.ui.Bg
import app.wavy.android.ui.Bg3
import app.wavy.android.ui.FullPlayer
import app.wavy.android.ui.ImportDialog
import app.wavy.android.ui.LibraryScreen
import app.wavy.android.ui.MiniPlayer
import app.wavy.android.ui.Muted
import app.wavy.android.ui.PlaylistsScreen
import app.wavy.android.ui.AnimeScreen
import app.wavy.android.ui.ShazamDialog
import app.wavy.android.ui.SearchScreen
import app.wavy.android.ui.SettingsScreen
import app.wavy.android.ui.TextC
import app.wavy.android.ui.WavyTheme
import app.wavy.android.update.Updater
import java.io.File

class MainActivity : ComponentActivity() {
    /** Ссылка, пришедшая через «Поделиться» (YouTube, Spotify…) — откроет окно импорта. */
    private val sharedUrl = mutableStateOf<String?>(null)

    private val askMedia = registerForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { res ->
        if (res.values.any { it }) AppState.scanLocal(this) else AppState.say(L("Без доступа к файлам музыку с телефона не найти", "Without file access music on the phone can't be found"))
    }
    private val askNotifications = registerForActivityResult(ActivityResultContracts.RequestPermission()) {}
    private val askMic = registerForActivityResult(ActivityResultContracts.RequestPermission()) { ok ->
        if (ok) AppState.listen() else AppState.say(L("Без микрофона Shazam не услышит музыку", "Without the microphone Shazam can't hear the music"))
    }

    /** Shazam: разрешение на микрофон → слушать. */
    private fun shazam() {
        AppState.shazamOpen.value = true
        if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) == android.content.pm.PackageManager.PERMISSION_GRANTED) AppState.listen()
        else askMic.launch(Manifest.permission.RECORD_AUDIO)
    }
    private val pickImport = registerForActivityResult(ActivityResultContracts.OpenDocument()) { uri -> uri?.let(::importFile) }
    private val saveExport = registerForActivityResult(ActivityResultContracts.CreateDocument("application/json")) { uri ->
        uri?.let { runCatching { contentResolver.openOutputStream(it)?.use { o -> o.write(Sync.export().toByteArray()) } }.onSuccess { AppState.say(L("Экспорт сохранён — открой этот файл в Wavy на ПК", "Export saved — open this file in Wavy on the PC")) } }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        Store.init(this)
        PlayerConn.connect(this)
        if (Build.VERSION.SDK_INT >= 33) askNotifications.launch(Manifest.permission.POST_NOTIFICATIONS)
        if (Store.lib.value.settings.autoUpdate) AppState.checkUpdate(manual = false)
        handleIntent(intent)
        AppState.micRequest = ::shazam
        setContent { WavyTheme { Root() } }
    }

    override fun onNewIntent(intent: Intent) { super.onNewIntent(intent); handleIntent(intent) }

    override fun onStop() { PlayerConn.saveSession(); Store.flush(); super.onStop() }

    private fun handleIntent(i: Intent?) {
        when (i?.action) {
            Intent.ACTION_SEND -> {
                (i.getParcelableExtra<Uri>(Intent.EXTRA_STREAM))?.let { importFile(it); return }
                val text = i.getStringExtra(Intent.EXTRA_TEXT) ?: return
                // приложения шлют «Название https://…» — берём саму ссылку
                Regex("""(https?://\S+|spotify:\S+)""").find(text)?.value?.let { sharedUrl.value = it }
            }
            Intent.ACTION_VIEW -> i.data?.let { u -> if (u.scheme == "content" || u.scheme == "file") importFile(u) else sharedUrl.value = u.toString() }
        }
    }

    private fun importFile(uri: Uri) {
        val text = runCatching { contentResolver.openInputStream(uri)?.bufferedReader()?.readText() }.getOrNull()
        if (text != null && Sync.isExport(text)) AppState.importExport(text) else AppState.say(L("Это не файл экспорта Wavy", "This isn't a Wavy export file"))
    }

    private fun scan() {
        val perms = if (Build.VERSION.SDK_INT >= 33) arrayOf(Manifest.permission.READ_MEDIA_AUDIO, Manifest.permission.READ_MEDIA_VIDEO)
        else arrayOf(Manifest.permission.READ_EXTERNAL_STORAGE)
        askMedia.launch(perms)
    }

    private fun setFloating(on: Boolean) {
        if (on && !Settings.canDrawOverlays(this)) {
            AppState.say(L("Разреши Wavy «Поверх других приложений» и включи снова", "Allow Wavy “Display over other apps” and turn it on again"))
            startActivity(Intent(Settings.ACTION_MANAGE_OVERLAY_PERMISSION, Uri.parse("package:$packageName")))
            return
        }
        Store.settings { it.copy(floatingPlayer = on) }
        if (on) AppState.say(L("Мини-плеер появится, когда свернёшь Wavy", "The mini player will appear when you minimize Wavy"))
    }

    private fun installUpdate() {
        when (val u = AppState.update.value) {
            is AppState.Upd.Available -> AppState.downloadUpdate(this, u.rel)
            is AppState.Upd.Ready -> if (Updater.canInstall(this)) Updater.install(this, u.file) else { AppState.say(L("Разреши установку из Wavy и нажми «Установить» ещё раз", "Allow installs from Wavy and tap “Install” again")); Updater.openInstallPermission(this) }
            else -> {}
        }
    }

    @Composable
    private fun Root() {
        var tab by rememberSaveable { mutableStateOf(0) }
        var openPlaylist by rememberSaveable { mutableStateOf<String?>(null) }
        var openAnime by rememberSaveable { mutableStateOf<String?>(null) }
        val shazamOpen by AppState.shazamOpen.collectAsState()
        val remoteOpen by AppState.remoteOpen.collectAsState()
        // серия досмотрена (последние 10 %) — отмечаем
        LaunchedEffect(Unit) {
            while (true) {
                kotlinx.coroutines.delay(5000)
                val id = PlayerConn.state.value.currentId ?: continue
                val dur = PlayerConn.duration()
                if (dur > 60_000 && PlayerConn.position() > dur * 0.9 && Store.lib.value.anime.any { id in it.episodes && id !in it.watched }) Store.markWatched(id)
            }
        }
        var full by rememberSaveable { mutableStateOf(false) }
        var importFor by remember { mutableStateOf<Pair<String, String?>?>(null) } // (ссылка, плейлист)
        var addTo by remember { mutableStateOf<List<String>?>(null) }
        val snack = remember { SnackbarHostState() }
        val msg by AppState.message.collectAsState()
        val busy by AppState.busy.collectAsState()
        val shared by sharedUrl
        val upd by AppState.update.collectAsState()

        LaunchedEffect(msg) { msg?.let { snack.showSnackbar(it); AppState.message.value = null } }
        LaunchedEffect(shared) { shared?.let { importFor = it to null; sharedUrl.value = null } }
        LaunchedEffect(upd) { if (upd is AppState.Upd.Available) AppState.say(L("Вышла новая версия Wavy — Настройки → Обновления", "A new version of Wavy is out — Settings → Updates")) }
        BackHandler(full || openPlaylist != null || openAnime != null) { if (full) full = false else if (openAnime != null) openAnime = null else openPlaylist = null }

        Box(Modifier.fillMaxSize().background(Bg)) {
            Scaffold(
                containerColor = Bg,
                snackbarHost = { SnackbarHost(snack) },
                bottomBar = {
                    Column {
                        MiniPlayer { full = true }
                        NavigationBar(containerColor = Bg3) {
                            val items = listOf(L("Библиотека", "Library") to Icons.Rounded.LibraryMusic, L("Плейлисты", "Playlists") to Icons.AutoMirrored.Rounded.QueueMusic, L("Аниме", "Anime") to Icons.Rounded.LiveTv, L("Поиск", "Search") to Icons.Rounded.Search, L("Настройки", "Settings") to Icons.Rounded.Settings)
                            items.forEachIndexed { i, (label, icon) ->
                                NavigationBarItem(tab == i, { tab = i; if (i == 1) openPlaylist = null; if (i == 2) openAnime = null }, { Icon(icon, label) }, label = { Text(label, maxLines = 1, fontSize = 11.sp) },
                                    colors = NavigationBarItemDefaults.colors(selectedIconColor = Bg, indicatorColor = TextC, unselectedIconColor = Muted, unselectedTextColor = Muted))
                            }
                        }
                    }
                },
            ) { pad ->
                Column(Modifier.padding(pad).statusBarsPadding()) {
                    busy?.let { Text(it, color = Muted, modifier = Modifier.padding(horizontal = 16.dp)); LinearProgressIndicator(Modifier.padding(16.dp, 4.dp)) }
                    when (tab) {
                        0 -> LibraryScreen(onImport = { importFor = "" to null }, onScan = ::scan, onAddToPlaylist = { addTo = it })
                        1 -> PlaylistsScreen(openPlaylist, onOpen = { openPlaylist = it }, onImport = { importFor = "" to it }, onAddToPlaylist = { addTo = it })
                        2 -> AnimeScreen(openAnime, onOpen = { openAnime = it }, onScan = ::scan, onWatch = { full = true }, onShazam = ::shazam)
                        3 -> SearchScreen(onAddToPlaylist = { addTo = it })
                        else -> SettingsScreen(::setFloating, onExport = { saveExport.launch("wavy-export.json") }, onImportFile = { pickImport.launch(arrayOf("application/json", "text/*", "*/*")) }, onInstallUpdate = ::installUpdate)
                    }
                }
            }
            AnimatedVisibility(full, enter = slideInVertically { it }, exit = slideOutVertically { it }) { FullPlayer { full = false } }
            if (remoteOpen) {
                val st by Store.lib.collectAsState()
                BackHandler { AppState.remoteOpen.value = false }
                Box(Modifier.fillMaxSize().statusBarsPadding()) { app.wavy.android.ui.RemoteScreen(st.settings.syncAddr, st.settings.syncPin, back = { AppState.remoteOpen.value = false }) }
            }
        }
        importFor?.let { (url, pl) -> ImportDialog(url, pl, dismiss = { importFor = null }) { pid -> if (pid != null) { tab = 1; openPlaylist = pid } } }
        addTo?.let { ids -> AddToPlaylistDialog(ids) { addTo = null } }
        if (shazamOpen) ShazamDialog { AppState.shazamOpen.value = false; AppState.shazam.value = AppState.Shz.Idle }
    }
}
