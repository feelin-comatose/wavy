package app.wavy.android.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.wavy.android.data.L
import app.wavy.android.data.fmtTime

/** Manual video choice dialogs: list of Spotify tracks not found and video search (same as on the PC). */
@Composable
fun PickDialogs() {
    val nf by AppState.notFound.collectAsState()
    val pick by AppState.pick.collectAsState()
    nf?.let { list ->
        AlertDialog(onDismissRequest = { AppState.notFound.value = null },
            title = { Text(L("Не нашлось на YouTube: ${list.items.size}", "Not found on YouTube: ${list.items.size}")) },
            text = {
                Column {
                    Text(L("Нажми на трек и выбери нужное видео сам — он встанет в плейлист.", "Tap a track and pick the right video yourself — it goes into the playlist."), color = Muted, fontSize = 13.sp)
                    LazyColumn(Modifier.heightIn(max = 360.dp).padding(top = 8.dp)) {
                        itemsIndexed(list.items) { i, t ->
                            Column(Modifier.fillMaxWidth().clickable { AppState.pickForSpotify(list, i) }.padding(vertical = 8.dp)) {
                                Text(t.title, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                                Text(listOf(t.artists, t.album, if (t.duration > 0) fmtTime(t.duration) else "").filter { it.isNotEmpty() }.joinToString(" · "), color = Muted, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            }
                        }
                    }
                }
            },
            confirmButton = { TextButton({ AppState.notFound.value = null }) { Text(L("Закрыть", "Close")) } })
    }
    pick?.let { p ->
        var q by remember(p.replace, p.sp) { mutableStateOf(p.query) }
        AlertDialog(onDismissRequest = { AppState.pickCancel() },
            title = { Text(L("Выбери видео", "Pick a video")) },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    p.sp?.let { Text("${it.artists} — ${it.title}" + if (it.duration > 0) " · ${fmtTime(it.duration)}" else "", color = Muted, fontSize = 13.sp, maxLines = 2) }
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        OutlinedTextField(q, { q = it }, Modifier.weight(1f), singleLine = true)
                        TextButton({ if (q.isNotBlank()) AppState.pickSearch(p.copy(query = q.trim())) }, enabled = !p.busy) { Text(L("Найти", "Find")) }
                    }
                    p.error?.let { Text(it, color = Bad, fontSize = 13.sp) }
                    if (p.busy) Row(verticalAlignment = Alignment.CenterVertically) { CircularProgressIndicator(Modifier.padding(4.dp), color = Accent); Text(L("  Ищу в YouTube Music и YouTube…", "  Searching YouTube Music and YouTube…"), color = Muted) }
                    LazyColumn(Modifier.heightIn(max = 380.dp)) {
                        itemsIndexed(p.results) { _, c ->
                            Row(Modifier.fillMaxWidth().clickable { AppState.pickChoose(c) }.padding(vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
                                Cover(c.thumb, 48.dp)
                                Column(Modifier.weight(1f).padding(start = 10.dp)) {
                                    Text(c.title, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis, fontSize = 14.sp)
                                    val src = if (c.url?.contains("music.youtube") == true) "YouTube Music" else "YouTube"
                                    Text(listOf(c.artist.takeIf { it != "YouTube" } ?: "", c.album, if (c.duration > 0) fmtTime(c.duration) else "", src).filter { it.isNotEmpty() }.joinToString(" · "), color = Muted, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                                }
                            }
                        }
                    }
                }
            },
            confirmButton = { TextButton({ AppState.pickCancel() }) { Text(if (p.rest.isNotEmpty() || p.sp != null) L("Назад к списку", "Back to the list") else L("Отмена", "Cancel")) } })
    }
}
