package app.wavy.android.ui

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.automirrored.rounded.PlaylistAdd
import androidx.compose.material.icons.automirrored.rounded.QueueMusic
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.Delete
import androidx.compose.material.icons.rounded.DownloadForOffline
import androidx.compose.material.icons.rounded.Edit
import androidx.compose.material.icons.rounded.GraphicEq
import androidx.compose.material.icons.rounded.Link
import androidx.compose.material.icons.rounded.MoreVert
import androidx.compose.material.icons.rounded.PhoneAndroid
import androidx.compose.material.icons.rounded.PlayArrow
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material.icons.rounded.Shuffle
import androidx.compose.material.icons.rounded.Sync
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.wavy.android.data.Store
import app.wavy.android.data.Track
import app.wavy.android.data.fmtTime
import app.wavy.android.player.PlayerConn

// ---------- общие элементы ----------

@Composable
fun Header(title: String, sub: String? = null, back: (() -> Unit)? = null, actions: @Composable () -> Unit = {}) {
    Row(Modifier.fillMaxWidth().padding(start = if (back == null) 20.dp else 4.dp, end = 8.dp, top = 16.dp, bottom = 8.dp), verticalAlignment = Alignment.CenterVertically) {
        if (back != null) IconButton(onClick = back) { Icon(Icons.AutoMirrored.Rounded.ArrowBack, "Назад") }
        Column(Modifier.weight(1f)) {
            Text(title, fontSize = 28.sp, fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
            if (sub != null) Text(sub, color = Muted, fontSize = 13.sp)
        }
        actions()
    }
}

@Composable
fun Pill(text: String, icon: ImageVector? = null, primary: Boolean = false, onClick: () -> Unit) {
    val content: @Composable () -> Unit = {
        if (icon != null) { Icon(icon, null, Modifier.size(18.dp)); Spacer(Modifier.width(6.dp)) }
        Text(text, fontWeight = FontWeight.SemiBold, fontSize = 14.sp)
    }
    if (primary) Button(onClick, shape = RoundedCornerShape(99.dp), contentPadding = PaddingValues(horizontal = 16.dp, vertical = 8.dp)) { content() }
    else OutlinedButton(onClick, shape = RoundedCornerShape(99.dp), contentPadding = PaddingValues(horizontal = 14.dp, vertical = 8.dp),
        colors = ButtonDefaults.outlinedButtonColors(containerColor = Bg3, contentColor = TextC)) { content() }
}

@Composable
fun Empty(title: String, text: String, action: @Composable () -> Unit = {}) {
    Column(Modifier.fillMaxWidth().padding(top = 60.dp, start = 32.dp, end = 32.dp), horizontalAlignment = Alignment.CenterHorizontally) {
        Text(title, fontSize = 20.sp, fontWeight = FontWeight.Bold)
        Spacer(Modifier.height(6.dp))
        Text(text, color = Muted, fontSize = 14.sp, textAlign = androidx.compose.ui.text.style.TextAlign.Center)
        Spacer(Modifier.height(16.dp))
        action()
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
fun TrackRow(t: Track, index: Int?, playing: Boolean, onClick: () -> Unit, menu: @Composable (close: () -> Unit) -> Unit) {
    var open by remember { mutableStateOf(false) }
    Row(
        Modifier.fillMaxWidth().combinedClickable(onClick = onClick, onLongClick = { open = true }).padding(horizontal = 16.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (index != null) Box(Modifier.width(28.dp), contentAlignment = Alignment.Center) {
            if (playing) Icon(Icons.Rounded.GraphicEq, null, tint = Accent, modifier = Modifier.size(18.dp))
            else Text("${index + 1}", color = Muted, fontSize = 13.sp)
        }
        Cover(t.thumb, 48.dp, youtube = !t.isLocal)
        Column(Modifier.weight(1f).padding(start = 12.dp)) {
            Text(t.title, maxLines = 1, overflow = TextOverflow.Ellipsis, fontWeight = if (playing) FontWeight.Bold else FontWeight.Medium, fontSize = 15.sp)
            Text(listOf(t.artist, fmtTime(t.duration)).filter { it.isNotEmpty() }.joinToString(" · "), color = Muted, fontSize = 13.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        Box {
            IconButton(onClick = { open = true }) { Icon(Icons.Rounded.MoreVert, "Ещё", tint = Muted) }
            DropdownMenu(open, onDismissRequest = { open = false }) { menu { open = false } }
        }
    }
}

/** Меню трека: играть далее, в очередь, в плейлист, удалить. */
@Composable
fun TrackMenu(ids: List<String>, close: () -> Unit, playlistId: String? = null, onAddToPlaylist: (List<String>) -> Unit) {
    DropdownMenuItem({ Text("Играть далее") }, onClick = { PlayerConn.playNext(ids); AppState.say("Сыграет следующим"); close() })
    DropdownMenuItem({ Text("Добавить в очередь") }, onClick = { PlayerConn.enqueue(ids); AppState.say("Добавлено в очередь"); close() })
    DropdownMenuItem({ Text("Добавить в плейлист…") }, onClick = { close(); onAddToPlaylist(ids) })
    if (playlistId != null) DropdownMenuItem({ Text("Убрать из плейлиста") }, onClick = { Store.removeFromPlaylist(playlistId, ids.toSet()); close() })
    if (ids.all { Store.lib.value.tracks.containsKey(it) }) DropdownMenuItem({ Text("Удалить из библиотеки", color = Bad) }, onClick = { Store.removeTracks(ids.toSet()); close() })
    else DropdownMenuItem({ Text("В библиотеку") }, onClick = { Store.addTracks(ids.mapNotNull(Store::track)); AppState.say("Добавлено в библиотеку"); close() })
}

@Composable
fun TrackList(ids: List<String>, playlistId: String? = null, header: @Composable () -> Unit = {}, onAddToPlaylist: (List<String>) -> Unit) {
    val lib by Store.lib.collectAsState()
    val st by PlayerConn.state.collectAsState()
    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 16.dp)) {
        item { header() }
        itemsIndexed(ids, key = { i, id -> "$i/$id" }) { i, id ->
            val t = lib.tracks[id] ?: Store.track(id) ?: return@itemsIndexed
            TrackRow(t, i, st.currentId == id, onClick = { PlayerConn.play(ids, i) }) { close ->
                TrackMenu(listOf(id), close, playlistId, onAddToPlaylist)
            }
        }
    }
}

// ---------- экраны ----------

@Composable
fun LibraryScreen(onImport: () -> Unit, onScan: () -> Unit, onAddToPlaylist: (List<String>) -> Unit) {
    val lib by Store.lib.collectAsState()
    var filter by remember { mutableStateOf("") }
    val all = remember(lib) { Store.libraryIds(lib) }
    val ids = remember(all, filter) {
        val q = filter.trim().lowercase()
        if (q.isEmpty()) all else all.filter { id -> lib.tracks[id]?.let { "${it.title} ${it.artist} ${it.album}".lowercase().contains(q) } == true }
    }
    TrackList(ids, header = {
        Header("Библиотека", "${all.size} трек(ов)")
        LazyRow(contentPadding = PaddingValues(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            item { Pill("Слушать", Icons.Rounded.PlayArrow, primary = true) { PlayerConn.play(ids) } }
            item { Pill("Вперемешку", Icons.Rounded.Shuffle) { PlayerConn.play(ids, shuffle = true) } }
            item { Pill("С телефона", Icons.Rounded.PhoneAndroid) { onScan() } }
            item { Pill("По ссылке", Icons.Rounded.Link) { onImport() } }
        }
        if (all.isNotEmpty()) OutlinedTextField(filter, { filter = it }, Modifier.fillMaxWidth().padding(16.dp, 12.dp, 16.dp, 4.dp),
            placeholder = { Text("Фильтр по названию, артисту, альбому…") }, leadingIcon = { Icon(Icons.Rounded.Search, null) }, singleLine = true, shape = RoundedCornerShape(99.dp))
        if (all.isEmpty()) Empty("Тут пока тихо", "Найди музыку и видео на телефоне или вставь ссылку на плейлист YouTube Music / Spotify") {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) { Pill("С телефона", Icons.Rounded.PhoneAndroid, true) { onScan() }; Pill("По ссылке", Icons.Rounded.Link) { onImport() } }
        }
    }, onAddToPlaylist = onAddToPlaylist)
}

@Composable
fun PlaylistsScreen(openId: String?, onOpen: (String?) -> Unit, onImport: (String?) -> Unit, onAddToPlaylist: (List<String>) -> Unit) {
    val lib by Store.lib.collectAsState()
    var rename by remember { mutableStateOf<String?>(null) }
    var create by remember { mutableStateOf(false) }
    var delete by remember { mutableStateOf<String?>(null) }
    val p = lib.playlists.firstOrNull { it.id == openId }
    if (p == null) {
        LazyColumn(Modifier.fillMaxSize()) {
            item {
                Header("Плейлисты", "${lib.playlists.size}") { IconButton({ create = true }) { Icon(Icons.Rounded.Add, "Новый плейлист") } }
                Row(Modifier.padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Pill("Новый", Icons.Rounded.Add) { create = true }
                    Pill("Из YouTube / Spotify", Icons.Rounded.Link) { onImport(null) }
                }
                if (lib.playlists.isEmpty()) Empty("Плейлистов пока нет", "Создай свой или импортируй по ссылке — YouTube Music, YouTube, Spotify")
                Spacer(Modifier.height(8.dp))
            }
            items(lib.playlists, key = { it.id }) { pl ->
                val first = pl.trackIds.firstNotNullOfOrNull { lib.tracks[it]?.thumb }
                Row(Modifier.fillMaxWidth().clickable { onOpen(pl.id) }.padding(horizontal = 16.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                    Cover(first, 56.dp, 10.dp)
                    Column(Modifier.weight(1f).padding(start = 12.dp)) {
                        Text(pl.name, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Text("${pl.trackIds.size} трек(ов)" + if (pl.isRemote) " · импорт" else "", color = Muted, fontSize = 13.sp)
                    }
                    Icon(Icons.AutoMirrored.Rounded.QueueMusic, null, tint = Muted)
                }
            }
        }
    } else {
        val ids = p.trackIds.filter { lib.tracks.containsKey(it) }
        val offline by app.wavy.android.data.Offline.saved.collectAsState()
        TrackList(ids, playlistId = p.id, header = {
            Header(p.name, "${ids.size} трек(ов) · ${fmtTime(ids.sumOf { lib.tracks[it]?.duration ?: 0.0 })}", back = { onOpen(null) })
            LazyRow(contentPadding = PaddingValues(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                item { Pill("Слушать", Icons.Rounded.PlayArrow, primary = true) { PlayerConn.play(ids) } }
                item { Pill("Вперемешку", Icons.Rounded.Shuffle) { PlayerConn.play(ids, shuffle = true) } }
                if (p.isRemote) item { Pill("Синхронизировать", Icons.Rounded.Sync) { AppState.syncPlaylist(p.id) } }
                val yt = ids.mapNotNull { lib.tracks[it] }.filter { it.videoId != null }
                if (yt.isNotEmpty()) item {
                    val have = yt.count { it.videoId in offline }
                    Pill(if (have == yt.size) "Скачано" else if (have > 0) "Скачать ($have/${yt.size})" else "Скачать", Icons.Rounded.DownloadForOffline) { AppState.downloadOffline(ids) }
                }
                item { Pill("Добавить", Icons.Rounded.Link) { onImport(p.id) } }
                item { Pill("Переименовать", Icons.Rounded.Edit) { rename = p.id } }
                item { Pill("Удалить", Icons.Rounded.Delete) { delete = p.id } }
            }
            if (ids.isEmpty()) Empty("Плейлист пустой", "Добавляй треки через меню ⋮ в библиотеке или поиске")
            Spacer(Modifier.height(8.dp))
        }, onAddToPlaylist = onAddToPlaylist)
    }
    if (create) TextDialog("Новый плейлист", "", "Создать", { create = false }) { name -> onOpen(Store.createPlaylist(name).id) }
    rename?.let { id -> TextDialog("Переименовать", lib.playlists.firstOrNull { it.id == id }?.name ?: "", "Сохранить", { rename = null }) { Store.renamePlaylist(id, it) } }
    delete?.let { id ->
        AlertDialog(onDismissRequest = { delete = null }, title = { Text("Удалить «${lib.playlists.firstOrNull { it.id == id }?.name}»?") },
            text = { Text("Треки останутся в библиотеке.") },
            confirmButton = { TextButton({ Store.deletePlaylist(id); delete = null; onOpen(null) }) { Text("Удалить", color = Bad) } },
            dismissButton = { TextButton({ delete = null }) { Text("Отмена") } })
    }
}

@Composable
fun SearchScreen(onAddToPlaylist: (List<String>) -> Unit) {
    val q by AppState.searchQuery.collectAsState()
    val music by AppState.searchMusic.collectAsState()
    val results by AppState.searchResults.collectAsState()
    val searching by AppState.searching.collectAsState()
    val lib by Store.lib.collectAsState()
    val st by PlayerConn.state.collectAsState()
    LazyColumn(Modifier.fillMaxSize()) {
        item {
            Header("Поиск", "YouTube Music и YouTube")
            OutlinedTextField(q, { AppState.searchQuery.value = it }, Modifier.fillMaxWidth().padding(horizontal = 16.dp),
                placeholder = { Text("Трек, артист — или вставь ссылку") }, leadingIcon = { Icon(Icons.Rounded.Search, null) }, singleLine = true,
                shape = RoundedCornerShape(99.dp), keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                keyboardActions = KeyboardActions(onSearch = { AppState.search() }),
                trailingIcon = { if (searching) CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp) else TextButton({ AppState.search() }) { Text("Найти") } })
            Row(Modifier.padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                FilterChip(music, { AppState.searchMusic.value = true; if (q.isNotBlank()) AppState.search() }, { Text("Треки") })
                FilterChip(!music, { AppState.searchMusic.value = false; if (q.isNotBlank()) AppState.search() }, { Text("Видео") })
            }
            if (results.isEmpty() && !searching) Empty("Найди что-нибудь", "Например: «lofi hip hop», «Daft Punk», или вставь ссылку на плейлист")
        }
        itemsIndexed(results, key = { _, t -> t.id }) { i, t ->
            val inLib = lib.tracks.containsKey(t.id)
            Row(verticalAlignment = Alignment.CenterVertically) {
                Box(Modifier.weight(1f)) {
                    TrackRow(t, null, st.currentId == t.id, onClick = { Store.remember(results); PlayerConn.play(results.map { it.id }, i) }) { close ->
                        TrackMenu(listOf(t.id), close, onAddToPlaylist = { Store.remember(listOf(t)); onAddToPlaylist(it) })
                    }
                }
                if (!inLib) IconButton({ Store.addTracks(listOf(t)); AppState.say("Добавлено в библиотеку") }, Modifier.padding(end = 4.dp)) { Icon(Icons.Rounded.Add, "В библиотеку") }
            }
        }
    }
}

// ---------- диалоги ----------

@Composable
fun TextDialog(title: String, initial: String, ok: String, dismiss: () -> Unit, onOk: (String) -> Unit) {
    var v by remember { mutableStateOf(initial) }
    AlertDialog(onDismissRequest = dismiss, title = { Text(title) },
        text = { OutlinedTextField(v, { v = it }, singleLine = true, modifier = Modifier.fillMaxWidth()) },
        confirmButton = { TextButton({ if (v.isNotBlank()) { onOk(v.trim()); dismiss() } }) { Text(ok) } },
        dismissButton = { TextButton(dismiss) { Text("Отмена") } })
}

@Composable
fun ImportDialog(initial: String, intoPlaylist: String?, dismiss: () -> Unit, opened: (String?) -> Unit) {
    var url by remember { mutableStateOf(initial) }
    AlertDialog(onDismissRequest = dismiss, title = { Text("Импорт по ссылке") },
        text = {
            Column {
                Text("Spotify (плейлист, альбом, трек), YouTube Music, YouTube, SoundCloud, Bandcamp. Можно и через «Поделиться» прямо из их приложений.", color = Muted, fontSize = 13.sp)
                Spacer(Modifier.height(10.dp))
                OutlinedTextField(url, { url = it }, singleLine = true, modifier = Modifier.fillMaxWidth(), placeholder = { Text("https://…") })
            }
        },
        confirmButton = { TextButton({ if (url.trim().startsWith("http") || url.trim().startsWith("spotify:")) { AppState.importUrl(url.trim(), intoPlaylist, opened); dismiss() } else AppState.say("Нужна ссылка") }) { Text("Импортировать") } },
        dismissButton = { TextButton(dismiss) { Text("Отмена") } })
}

@Composable
fun AddToPlaylistDialog(ids: List<String>, dismiss: () -> Unit) {
    val lib by Store.lib.collectAsState()
    var create by remember { mutableStateOf(false) }
    AlertDialog(onDismissRequest = dismiss, title = { Text("Добавить в плейлист") },
        text = {
            LazyColumn {
                item { Row(Modifier.fillMaxWidth().clickable { create = true }.padding(vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                    Icon(Icons.AutoMirrored.Rounded.PlaylistAdd, null); Spacer(Modifier.width(12.dp)); Text("Новый плейлист…") } }
                item { HorizontalDivider() }
                items(lib.playlists, key = { it.id }) { p ->
                    Text(p.name, Modifier.fillMaxWidth().clickable {
                        Store.addTracks(ids.mapNotNull(Store::track).filter { !lib.tracks.containsKey(it.id) })
                        Store.addToPlaylist(p.id, ids); AppState.say("Добавлено в «${p.name}»"); dismiss()
                    }.padding(vertical = 12.dp))
                }
            }
        },
        confirmButton = { TextButton(dismiss) { Text("Закрыть") } })
    if (create) TextDialog("Новый плейлист", "", "Создать", { create = false }) { name ->
        Store.addTracks(ids.mapNotNull(Store::track).filter { !lib.tracks.containsKey(it.id) })
        Store.createPlaylist(name, ids); AppState.say("Плейлист «$name» создан"); dismiss()
    }
}

@Composable
fun Section(title: String, content: @Composable () -> Unit) {
    Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp).clip(RoundedCornerShape(16.dp)).background(Bg2).padding(16.dp)) {
        Text(title, fontWeight = FontWeight.Bold, fontSize = 16.sp)
        Spacer(Modifier.height(8.dp))
        content()
    }
}
