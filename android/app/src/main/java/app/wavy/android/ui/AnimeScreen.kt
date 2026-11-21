package app.wavy.android.ui

import android.content.Intent
import android.net.Uri
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.OpenInNew
import androidx.compose.material.icons.rounded.CheckCircle
import androidx.compose.material.icons.rounded.GraphicEq
import androidx.compose.material.icons.rounded.LiveTv
import androidx.compose.material.icons.rounded.PhoneAndroid
import androidx.compose.material.icons.rounded.PlayArrow
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.wavy.android.data.Anime
import app.wavy.android.data.Store
import app.wavy.android.data.fmtTime
import app.wavy.android.player.PlayerConn
import coil3.compose.AsyncImage

/** Раздел «Аниме»: тайтлы из видео на телефоне, постеры и названия с Шикимори. */
@Composable
fun AnimeScreen(openId: String?, onOpen: (String?) -> Unit, onScan: () -> Unit, onWatch: () -> Unit, onShazam: () -> Unit) {
    val lib by Store.lib.collectAsState()
    val open = openId?.let { id -> lib.anime.firstOrNull { it.id == id } }
    if (open != null) return AnimeTitle(open, back = { onOpen(null) }, onWatch = onWatch)
    Column(Modifier.fillMaxSize()) {
        Header("Аниме", if (lib.anime.isEmpty()) null else "${lib.anime.size} тайтл(ов)") {
            IconButton(onShazam) { Icon(Icons.Rounded.GraphicEq, "Что за песня? (Shazam)") }
            IconButton(onScan) { Icon(Icons.Rounded.PhoneAndroid, "Найти серии на телефоне") }
        }
        if (lib.anime.isEmpty()) {
            Empty("Пока пусто", "Положи серии (mp4, mkv) в память телефона — например, в папку с названием аниме — и нажми «Найти на телефоне». Номер серии Wavy поймёт из имени файла, обложку и название возьмёт с Шикимори.") {
                Pill("Найти на телефоне", Icons.Rounded.Search, primary = true) { onScan() }
            }
            return@Column
        }
        LazyVerticalGrid(GridCells.Adaptive(150.dp), contentPadding = PaddingValues(12.dp), horizontalArrangement = Arrangement.spacedBy(12.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
            items(lib.anime.sortedByDescending { it.addedAt }, key = { it.id }) { a ->
                Column(Modifier.clickable { onOpen(a.id) }) {
                    Box {
                        Poster(a, Modifier.fillMaxWidth().aspectRatio(2f / 3f))
                        Text("${a.watched.size}/${a.episodes.size}", Modifier.align(Alignment.TopEnd).padding(6.dp).background(Bg.copy(alpha = 0.8f), RoundedCornerShape(99.dp)).padding(horizontal = 8.dp, vertical = 2.dp), fontSize = 11.sp, fontWeight = FontWeight.Bold)
                    }
                    Text(a.display, Modifier.padding(top = 6.dp), fontWeight = FontWeight.SemiBold, fontSize = 14.sp, maxLines = 2, overflow = TextOverflow.Ellipsis)
                    if (a.meta.isNotEmpty()) Text(a.meta, color = Muted, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
        }
    }
}

@Composable
private fun Poster(a: Anime, modifier: Modifier) {
    Box(modifier.clip(RoundedCornerShape(12.dp)).background(Bg3), contentAlignment = Alignment.Center) {
        if (a.poster != null) AsyncImage(a.poster, a.display, Modifier.fillMaxSize(), contentScale = androidx.compose.ui.layout.ContentScale.Crop)
        else Icon(Icons.Rounded.LiveTv, null, tint = Muted, modifier = Modifier.size(40.dp))
    }
}

@Composable
private fun AnimeTitle(a: Anime, back: () -> Unit, onWatch: () -> Unit) {
    val st by PlayerConn.state.collectAsState()
    val ctx = LocalContext.current
    LazyColumn(Modifier.fillMaxSize()) {
        item { Header(a.display, back = back) }
        item {
            Row(Modifier.padding(horizontal = 16.dp)) {
                Poster(a, Modifier.width(120.dp).aspectRatio(2f / 3f))
                Column(Modifier.padding(start = 14.dp)) {
                    if (a.name.isNotEmpty() && a.name != a.display) Text(a.name, color = Muted, fontSize = 13.sp)
                    if (a.meta.isNotEmpty()) Text(a.meta, color = Muted, fontSize = 13.sp)
                    Text("серий: ${a.episodes.size} · просмотрено ${a.watched.size}", color = Muted, fontSize = 13.sp)
                    Spacer(Modifier.height(10.dp))
                    val next = a.last?.let { l -> a.episodes.indexOf(l).takeIf { it >= 0 } }
                    Pill(if (next != null) "Продолжить" else "Смотреть", Icons.Rounded.PlayArrow, primary = true) { AppState.playAnime(a.id); onWatch() }
                    a.url?.let { u -> Spacer(Modifier.height(6.dp)); Pill("Шикимори", Icons.AutoMirrored.Rounded.OpenInNew) { ctx.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(u))) } }
                }
            }
            Spacer(Modifier.height(12.dp))
        }
        itemsIndexed(a.episodes) { _, id ->
            val t = Store.track(id) ?: return@itemsIndexed
            val seen = id in a.watched
            val playing = st.currentId == id
            Row(Modifier.fillMaxWidth().clickable { AppState.playAnime(a.id, id); onWatch() }.padding(horizontal = 16.dp, vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(t.title, color = if (playing) Accent else if (seen) Muted else TextC, fontWeight = if (playing) FontWeight.Bold else FontWeight.Medium)
                    Text(fmtTime(t.duration), color = Muted, fontSize = 12.sp)
                }
                if (seen) Icon(Icons.Rounded.CheckCircle, "Просмотрено", tint = Muted, modifier = Modifier.size(18.dp))
            }
        }
    }
}

/** Окно Shazam: слушаем микрофон, показываем, что нашлось, добавляем в «Опенинги». */
@Composable
fun ShazamDialog(dismiss: () -> Unit) {
    val s by AppState.shazam.collectAsState()
    AlertDialog(
        onDismissRequest = dismiss, containerColor = Bg2,
        title = { Text("Что за песня?") },
        text = {
            when (val v = s) {
                AppState.Shz.Listening -> Row(verticalAlignment = Alignment.CenterVertically) { CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp); Text("   Слушаю 12 секунд…") }
                AppState.Shz.Adding -> Row(verticalAlignment = Alignment.CenterVertically) { CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp); Text("   Ищу на YouTube…") }
                is AppState.Shz.Found -> Row(verticalAlignment = Alignment.CenterVertically) {
                    Cover(v.f.cover, 64.dp)
                    Column(Modifier.padding(start = 12.dp)) { Text(v.f.title, fontWeight = FontWeight.Bold); Text(v.f.artist, color = Muted) }
                }
                AppState.Shz.NotFound -> Text("Не узнал. Поднеси телефон ближе к звуку и попробуй ещё раз.", color = Muted)
                is AppState.Shz.Error -> Text(v.msg, color = Bad)
                AppState.Shz.Idle -> Text("Поднеси телефон к звуку — например, к опенингу аниме.", color = Muted)
            }
        },
        confirmButton = {
            when (val v = s) {
                is AppState.Shz.Found -> TextButton({ AppState.addFound(v.f) }) { Text("В «Опенинги»") }
                AppState.Shz.Listening, AppState.Shz.Adding -> {}
                else -> TextButton({ AppState.startShazam() }) { Text(if (s == AppState.Shz.Idle) "Слушать" else "Ещё раз") }
            }
        },
        dismissButton = { TextButton(dismiss) { Text("Закрыть") } },
    )
}
