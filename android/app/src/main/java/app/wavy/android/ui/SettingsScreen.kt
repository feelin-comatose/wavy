package app.wavy.android.ui

import app.wavy.android.data.L
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
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
import androidx.compose.material.icons.rounded.SettingsRemote
import androidx.compose.material.icons.rounded.Sync
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
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.foundation.clickable
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.wavy.android.data.Store
import app.wavy.android.update.Updater

@OptIn(ExperimentalLayoutApi::class)
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
        Header(L("Настройки", "Settings"))

        Section("Язык · Language") {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                listOf("ru" to "Русский", "en" to "English").forEach { (code, name) ->
                    Pill(name, primary = s.lang == code) { Store.settings { it.copy(lang = code) } }
                }
            }
        }

        Section(L("Тема", "Theme")) {
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Themes.list.forEach { (id, names, _) ->
                    Pill(L(names.first, names.second), primary = s.theme == id) { Store.settings { it.copy(theme = id) } }
                }
            }
            Spacer(Modifier.height(12.dp))
            Text(L("Цвет акцента", "Accent color"), color = Muted, fontSize = 13.sp)
            Spacer(Modifier.height(6.dp))
            FlowRow(horizontalArrangement = Arrangement.spacedBy(10.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                // the theme's own accent, then the swatches
                val own = Themes.list.firstOrNull { it.first == s.theme }?.third?.accent ?: Accent
                Swatch(own, s.accent.isEmpty(), L("Как в теме", "Theme default")) { Store.settings { it.copy(accent = "") } }
                Themes.accents.forEach { hex -> Swatch(Themes.parseHex(hex)!!, s.accent.equals(hex, true), hex) { Store.settings { it.copy(accent = hex) } } }
            }
            Spacer(Modifier.height(8.dp))
            var hex by remember(s.accent) { mutableStateOf(s.accent) }
            OutlinedTextField(hex, { v -> hex = v.take(7); if (Themes.parseHex(hex) != null) Store.settings { it.copy(accent = "#" + hex.trim().removePrefix("#").lowercase()) } },
                label = { Text(L("Свой цвет: #rrggbb", "Custom color: #rrggbb")) }, singleLine = true, modifier = Modifier.fillMaxWidth())
        }

        Section(L("Мини-плеер поверх приложений", "Mini player over apps")) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(L("Показывать плавающий плеер, когда Wavy свёрнут (поверх игр и других приложений)", "Show a floating player when Wavy is minimized (over games and other apps)"), Modifier.weight(1f), color = Muted, fontSize = 13.sp)
                Switch(s.floatingPlayer, onFloating, colors = SwitchDefaults.colors(checkedTrackColor = Accent, checkedThumbColor = OnAccent))
            }
        }

        Section(L("Синхронизация с ПК", "Sync with PC")) {
            Text(L("Плейлисты и треки YouTube / Spotify переносятся файлом wavy-export.json в обе стороны: «Экспорт» здесь → открыть файл в Wavy на ПК (Настройки → Синхронизация), и наоборот. Локальные файлы не переносятся.", "Playlists and YouTube / Spotify tracks are transferred in both directions with the wavy-export.json file: “Export” here → open the file in Wavy on the PC (Settings → Sync), and the other way round. Local files aren't transferred."), color = Muted, fontSize = 13.sp)
            Spacer(Modifier.height(10.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Pill(L("Экспорт", "Export"), Icons.Rounded.Upload, primary = true) { onExport() }
                Pill(L("Импорт файла", "Import file"), Icons.Rounded.Download) { onImportFile() }
            }
            Spacer(Modifier.height(14.dp))
            Text(L("По Wi-Fi, без файлов: в Wavy на ПК включи Настройки → Синхронизация → «По Wi-Fi» и впиши сюда адрес и PIN оттуда. Телефон и ПК — в одной сети.", "Over Wi-Fi, without files: in Wavy on the PC turn on Settings → Sync → “Over Wi-Fi” and enter the address and PIN from there. The phone and the PC must be on the same network."), color = Muted, fontSize = 13.sp)
            var addr by remember(s.syncAddr) { mutableStateOf(s.syncAddr) }
            var pin by remember(s.syncPin) { mutableStateOf(s.syncPin) }
            OutlinedTextField(addr, { v -> addr = v.trim(); Store.settings { it.copy(syncAddr = addr) } }, Modifier.fillMaxWidth().padding(top = 8.dp), label = { Text(L("Адрес ПК (например 192.168.1.5:3738)", "PC address (e.g. 192.168.1.5:3738)")) }, singleLine = true)
            OutlinedTextField(pin, { v -> pin = v.filter { it.isDigit() }.take(6); Store.settings { it.copy(syncPin = pin) } }, Modifier.fillMaxWidth().padding(top = 8.dp), label = { Text("PIN") }, singleLine = true)
            Spacer(Modifier.height(8.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Pill(L("Синхронизировать", "Sync"), Icons.Rounded.Sync, primary = true) { if (addr.isBlank() || pin.length < 6) AppState.say(L("Впиши адрес ПК и PIN из 6 цифр", "Enter the PC address and the 6-digit PIN")) else AppState.wifiSync(addr, pin) }
                Pill(L("Пульт ПК", "PC remote"), Icons.Rounded.SettingsRemote) { if (addr.isBlank() || pin.length < 6) AppState.say(L("Впиши адрес ПК и PIN из 6 цифр", "Enter the PC address and the 6-digit PIN")) else AppState.remoteOpen.value = true }
            }
        }

        Section(L("Без интернета", "Offline")) {
            val saved by app.wavy.android.data.Offline.saved.collectAsState()
            val mb = remember(saved) { app.wavy.android.data.Offline.size() / 1_048_576 }
            Text(L("Треки с YouTube можно скачать на телефон: кнопка «Скачать» в плейлисте. Скачанное играет без интернета, например в метро.", "YouTube tracks can be downloaded to the phone: the “Download” button in a playlist. Downloaded tracks play without internet, e.g. in the subway."), color = Muted, fontSize = 13.sp)
            Spacer(Modifier.height(8.dp))
            Text(L("Скачано: ${saved.size} трек(ов), $mb МБ", "Downloaded: ${saved.size} track(s), $mb MB"), fontSize = 14.sp)
            if (saved.isNotEmpty()) { Spacer(Modifier.height(8.dp)); Pill(L("Удалить скачанное", "Delete downloads")) { app.wavy.android.data.Offline.clear() } }
        }

        Section("Spotify") {
            Text(L("Ссылки Spotify работают без настроек (до 100 треков из плейлиста). Для больших плейлистов: developer.spotify.com → Create app → Client ID и Client Secret.", "Spotify links work without setup (up to 100 tracks from a playlist). For big playlists: developer.spotify.com → Create app → Client ID and Client Secret."), color = Muted, fontSize = 13.sp)
            var id by remember(s.spotifyClientId) { mutableStateOf(s.spotifyClientId) }
            var sec by remember(s.spotifyClientSecret) { mutableStateOf(s.spotifyClientSecret) }
            OutlinedTextField(id, { v -> id = v.filter { it.isLetterOrDigit() }; Store.settings { it.copy(spotifyClientId = id) } }, Modifier.fillMaxWidth().padding(top = 8.dp), label = { Text("Client ID") }, singleLine = true)
            OutlinedTextField(sec, { v -> sec = v.filter { it.isLetterOrDigit() }; Store.settings { it.copy(spotifyClientSecret = sec) } }, Modifier.fillMaxWidth().padding(top = 8.dp), label = { Text("Client Secret") }, singleLine = true, visualTransformation = PasswordVisualTransformation())
        }

        Section(L("Обновления", "Updates")) {
            Text(L("Версия: сборка ${Updater.currentBuild}", "Version: build ${Updater.currentBuild}"), fontSize = 14.sp)
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(L("Проверять обновления при запуске", "Check for updates on launch"), Modifier.weight(1f), color = Muted, fontSize = 13.sp)
                Switch(s.autoUpdate, { v -> Store.settings { it.copy(autoUpdate = v) } }, colors = SwitchDefaults.colors(checkedTrackColor = Accent, checkedThumbColor = OnAccent))
            }
            Spacer(Modifier.height(6.dp))
            when (val u = upd) {
                AppState.Upd.Checking -> Row(verticalAlignment = Alignment.CenterVertically) { CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp); Text(L("  Проверяю…", "  Checking…"), color = Muted) }
                AppState.Upd.UpToDate -> Text(L("У тебя последняя версия", "You have the latest version"), color = Muted)
                is AppState.Upd.Available -> Pill(L("Скачать сборку ${u.rel.build}", "Download build ${u.rel.build}"), Icons.Rounded.SystemUpdate, primary = true) { onInstallUpdate() }
                is AppState.Upd.Downloading -> Text(L("Скачиваю… ${u.pct}%", "Downloading… ${u.pct}%"), color = Muted)
                is AppState.Upd.Ready -> Pill(L("Установить", "Install"), Icons.Rounded.SystemUpdate, primary = true) { onInstallUpdate() }
                is AppState.Upd.Error -> Text(u.msg, color = Bad, fontSize = 13.sp)
                AppState.Upd.Idle -> {}
            }
            if (upd !is AppState.Upd.Available && upd !is AppState.Upd.Ready && upd !is AppState.Upd.Downloading)
                Pill(L("Проверить обновления", "Check for updates"), Icons.Rounded.SystemUpdate) { AppState.checkUpdate(manual = true) }
        }
        Spacer(Modifier.height(24.dp))
    }
}

/** A round color swatch; the chosen one gets a ring in the text color. */
@Composable
private fun Swatch(color: androidx.compose.ui.graphics.Color, selected: Boolean, label: String, onClick: () -> Unit) {
    androidx.compose.foundation.layout.Box(
        Modifier.size(34.dp).clip(androidx.compose.foundation.shape.CircleShape)
            .border(if (selected) 3.dp else 1.dp, if (selected) TextC else Line, androidx.compose.foundation.shape.CircleShape)
            .padding(if (selected) 5.dp else 0.dp).clip(androidx.compose.foundation.shape.CircleShape)
            .background(color).clickable(onClickLabel = label, onClick = onClick)
            .semantics { contentDescription = label }
    )
}
