package app.wavy.android.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Download
import androidx.compose.material.icons.rounded.SystemUpdate
import androidx.compose.material.icons.rounded.Upload
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.wavy.android.data.Store
import app.wavy.android.update.Updater

@Composable
fun SettingsScreen(
    onFloating: (Boolean) -> Unit,
    onExport: () -> Unit,
    onImportFile: () -> Unit,
    onInstallUpdate: () -> Unit,
) {
    val lib by Store.lib.collectAsState()
    val s = lib.settings
    val upd by AppState.update.collectAsState()
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
        Header("Настройки")

        Section("Мини-плеер поверх приложений") {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Показывать плавающий плеер, когда Wavy свёрнут (поверх игр и других приложений)", Modifier.weight(1f), color = Muted, fontSize = 13.sp)
                Switch(s.floatingPlayer, onFloating, colors = SwitchDefaults.colors(checkedTrackColor = Accent, checkedThumbColor = Bg))
            }
        }

        Section("Синхронизация с ПК") {
            Text("Плейлисты и треки YouTube / Spotify переносятся файлом wavy-export.json в обе стороны: «Экспорт» здесь → открыть файл в Wavy на ПК (Настройки → Синхронизация), и наоборот. Локальные файлы не переносятся.", color = Muted, fontSize = 13.sp)
            Spacer(Modifier.height(10.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Pill("Экспорт", Icons.Rounded.Upload, primary = true) { onExport() }
                Pill("Импорт файла", Icons.Rounded.Download) { onImportFile() }
            }
        }

        Section("Spotify") {
            Text("Ссылки Spotify работают без настроек (до 100 треков из плейлиста). Для больших плейлистов: developer.spotify.com → Create app → Client ID и Client Secret.", color = Muted, fontSize = 13.sp)
            var id by remember(s.spotifyClientId) { mutableStateOf(s.spotifyClientId) }
            var sec by remember(s.spotifyClientSecret) { mutableStateOf(s.spotifyClientSecret) }
            OutlinedTextField(id, { v -> id = v.filter { it.isLetterOrDigit() }; Store.settings { it.copy(spotifyClientId = id) } }, Modifier.fillMaxWidth().padding(top = 8.dp), label = { Text("Client ID") }, singleLine = true)
            OutlinedTextField(sec, { v -> sec = v.filter { it.isLetterOrDigit() }; Store.settings { it.copy(spotifyClientSecret = sec) } }, Modifier.fillMaxWidth().padding(top = 8.dp), label = { Text("Client Secret") }, singleLine = true, visualTransformation = PasswordVisualTransformation())
        }

        Section("Обновления") {
            Text("Версия: сборка ${Updater.currentBuild}", fontSize = 14.sp)
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Проверять обновления при запуске", Modifier.weight(1f), color = Muted, fontSize = 13.sp)
                Switch(s.autoUpdate, { v -> Store.settings { it.copy(autoUpdate = v) } }, colors = SwitchDefaults.colors(checkedTrackColor = Accent, checkedThumbColor = Bg))
            }
            Spacer(Modifier.height(6.dp))
            when (val u = upd) {
                AppState.Upd.Checking -> Row(verticalAlignment = Alignment.CenterVertically) { CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp); Text("  Проверяю…", color = Muted) }
                AppState.Upd.UpToDate -> Text("У тебя последняя версия", color = Muted)
                is AppState.Upd.Available -> Pill("Скачать сборку ${u.rel.build}", Icons.Rounded.SystemUpdate, primary = true) { onInstallUpdate() }
                is AppState.Upd.Downloading -> Text("Скачиваю… ${u.pct}%", color = Muted)
                is AppState.Upd.Ready -> Pill("Установить", Icons.Rounded.SystemUpdate, primary = true) { onInstallUpdate() }
                is AppState.Upd.Error -> Text(u.msg, color = Bad, fontSize = 13.sp)
                AppState.Upd.Idle -> {}
            }
            if (upd !is AppState.Upd.Available && upd !is AppState.Upd.Ready && upd !is AppState.Upd.Downloading)
                Pill("Проверить обновления", Icons.Rounded.SystemUpdate) { AppState.checkUpdate(manual = true) }
        }
        Spacer(Modifier.height(24.dp))
    }
}
