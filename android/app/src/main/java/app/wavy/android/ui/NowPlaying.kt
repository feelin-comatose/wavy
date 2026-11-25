package app.wavy.android.ui

import app.wavy.android.data.L
import android.view.ViewGroup
import androidx.annotation.OptIn
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.QueueMusic
import androidx.compose.material.icons.rounded.Image
import androidx.compose.material.icons.rounded.Bedtime
import androidx.compose.material.icons.rounded.Favorite
import androidx.compose.material.icons.rounded.FavoriteBorder
import androidx.compose.material.icons.rounded.Lyrics
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material.icons.rounded.KeyboardArrowDown
import androidx.compose.material.icons.rounded.Pause
import androidx.compose.material.icons.rounded.PlayArrow
import androidx.compose.material.icons.rounded.Repeat
import androidx.compose.material.icons.rounded.RepeatOne
import androidx.compose.material.icons.rounded.Shuffle
import androidx.compose.material.icons.rounded.SkipNext
import androidx.compose.material.icons.rounded.SkipPrevious
import androidx.compose.material.icons.rounded.Videocam
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.material.icons.rounded.GraphicEq
import androidx.compose.ui.viewinterop.AndroidView
import androidx.media3.common.MediaItem
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.ui.AspectRatioFrameLayout
import androidx.media3.ui.PlayerView
import app.wavy.android.data.Store
import app.wavy.android.data.Track
import app.wavy.android.data.fmtTime
import app.wavy.android.player.PlayerConn
import app.wavy.android.yt.YouTube
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext

/** Позиция воспроизведения, обновляется 4 раза в секунду, пока экран видим. */
@Composable
fun rememberPosition(): Pair<Long, Long> {
    var pos by remember { mutableLongStateOf(0L) }
    var dur by remember { mutableLongStateOf(0L) }
    LaunchedEffect(Unit) { while (true) { pos = PlayerConn.position(); dur = PlayerConn.duration(); delay(250) } }
    return pos to dur
}

@Composable
fun MiniPlayer(onExpand: () -> Unit) {
    val st by PlayerConn.state.collectAsState()
    val t = st.currentId?.let(Store::track) ?: return
    val (pos, dur) = rememberPosition()
    Column(Modifier.fillMaxWidth().background(Bg2).clickable(onClick = onExpand)) {
        LinearProgressIndicator({ if (dur > 0) pos.toFloat() / dur else 0f }, Modifier.fillMaxWidth().height(2.dp), color = Accent, trackColor = Line, drawStopIndicator = {})
        Row(Modifier.padding(horizontal = 12.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            Cover(t.thumb, 44.dp)
            Column(Modifier.weight(1f).padding(horizontal = 12.dp)) {
                Text(t.title, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis, fontSize = 14.sp)
                Text(if (st.buffering) L("загрузка…", "loading…") else t.artist, color = Muted, maxLines = 1, overflow = TextOverflow.Ellipsis, fontSize = 12.sp)
            }
            IconButton({ PlayerConn.toggle() }) { Icon(if (st.isPlaying) Icons.Rounded.Pause else Icons.Rounded.PlayArrow, L("Играть/пауза", "Play/pause"), Modifier.size(30.dp)) }
            IconButton({ PlayerConn.next() }) { Icon(Icons.Rounded.SkipNext, L("Следующий", "Next")) }
        }
    }
}

@Composable
fun FullPlayer(onClose: () -> Unit) {
    val st by PlayerConn.state.collectAsState()
    val t = st.currentId?.let(Store::track)
    var showVideo by remember { mutableStateOf(true) }
    var showQueue by remember { mutableStateOf(false) }
    var showLyrics by remember { mutableStateOf(false) }
    var sleepMenu by remember { mutableStateOf(false) }
    val sleep by AppState.sleep.collectAsState()
    val lib by Store.lib.collectAsState()
    val (pos, dur) = rememberPosition()
    var drag by remember { mutableStateOf<Float?>(null) }
    Column(Modifier.fillMaxSize().background(Bg).statusBarsPadding().navigationBarsPadding()) {
        Row(Modifier.fillMaxWidth().padding(8.dp), verticalAlignment = Alignment.CenterVertically) {
            IconButton(onClose) { Icon(Icons.Rounded.KeyboardArrowDown, L("Свернуть", "Collapse"), Modifier.size(30.dp)) }
            Text(L("Сейчас играет", "Now playing"), Modifier.weight(1f), color = Muted, fontSize = 13.sp, textAlign = androidx.compose.ui.text.style.TextAlign.Center)
            if (t?.hasVideo == true) IconButton({ showVideo = !showVideo }) { Icon(if (showVideo) Icons.Rounded.Image else Icons.Rounded.Videocam, L("Видео / обложка", "Video / cover")) }
            IconButton({ AppState.startShazam() }) { Icon(Icons.Rounded.GraphicEq, L("Что за песня? (Shazam)", "What's this song? (Shazam)"), tint = Muted) }
            Box {
                IconButton({ sleepMenu = true }) { Icon(Icons.Rounded.Bedtime, L("Таймер сна", "Sleep timer"), tint = if (sleep != null) Accent else Muted) }
                DropdownMenu(sleepMenu, onDismissRequest = { sleepMenu = false }) {
                    for (m in listOf(15, 30, 45, 60, 90)) DropdownMenuItem({ Text(L("через $m мин", "in $m min")) }, onClick = { AppState.setSleep(m); sleepMenu = false })
                    DropdownMenuItem({ Text(L("после этого трека", "after this track")) }, onClick = { AppState.setSleep(0); sleepMenu = false })
                    if (sleep != null) DropdownMenuItem({ Text(L("отменить таймер", "cancel the timer"), color = Bad) }, onClick = { AppState.setSleep(null); sleepMenu = false })
                }
            }
            IconButton({ showLyrics = !showLyrics; if (showLyrics) showQueue = false }) { Icon(Icons.Rounded.Lyrics, L("Текст песни", "Lyrics"), tint = if (showLyrics) Accent else Muted) }
            IconButton({ showQueue = !showQueue; if (showQueue) showLyrics = false }) { Icon(Icons.AutoMirrored.Rounded.QueueMusic, L("Очередь", "Queue"), tint = if (showQueue) Accent else Muted) }
        }
        if (t == null) { Empty(L("Ничего не играет", "Nothing is playing"), L("Выбери трек в библиотеке", "Pick a track in the library")); return@Column }
        if (showLyrics) {
            Box(Modifier.weight(1f).fillMaxWidth()) { LyricsView(t.id, pos) }
        } else if (showQueue) {
            LazyColumn(Modifier.weight(1f)) {
                itemsIndexed(st.queue) { i, id ->
                    val q = Store.track(id) ?: return@itemsIndexed
                    Row(Modifier.fillMaxWidth().clickable { PlayerConn.jump(i) }.padding(horizontal = 16.dp, vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
                        Cover(q.thumb, 40.dp)
                        Column(Modifier.padding(start = 12.dp)) {
                            Text(q.title, maxLines = 1, overflow = TextOverflow.Ellipsis, color = if (i == st.index) Accent else if (i < st.index) Muted else TextC, fontWeight = if (i == st.index) FontWeight.Bold else FontWeight.Normal)
                            Text(q.artist, color = Muted, fontSize = 12.sp, maxLines = 1)
                        }
                    }
                }
            }
        } else {
            Box(Modifier.weight(1f).fillMaxWidth().padding(horizontal = 20.dp), contentAlignment = Alignment.Center) {
                if (t.hasVideo && showVideo) VideoArea(t, st.isPlaying)
                else Cover(t.cover ?: t.thumb, 320.dp, 18.dp, modifier = Modifier.aspectRatio(1f))
            }
        }
        Column(Modifier.padding(horizontal = 24.dp, vertical = 12.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(t.title, fontSize = 22.sp, fontWeight = FontWeight.Bold, maxLines = 2, overflow = TextOverflow.Ellipsis)
                    Text(t.artist, color = Muted, fontSize = 15.sp, maxLines = 1)
                }
                val liked = t.id in lib.liked
                IconButton({ if (!lib.tracks.containsKey(t.id)) Store.addTracks(listOf(t)); Store.setLiked(listOf(t.id), !liked) }) {
                    Icon(if (liked) Icons.Rounded.Favorite else Icons.Rounded.FavoriteBorder, if (liked) L("Убрать из «Любимого»", "Remove from “Liked”") else L("В «Любимое»", "Add to “Liked”"), tint = if (liked) Accent else Muted)
                }
            }
            val st2 = st.error
            if (st2 != null) Text(L("Ошибка: $st2", "Error: $st2"), color = Bad, fontSize = 12.sp, maxLines = 2)
            Slider(
                value = drag ?: (if (dur > 0) pos.toFloat() / dur else 0f),
                onValueChange = { drag = it }, onValueChangeFinished = { drag?.let { PlayerConn.seek((it * dur).toLong()) }; drag = null },
                colors = SliderDefaults.colors(thumbColor = Accent, activeTrackColor = Accent, inactiveTrackColor = Line),
            )
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                Text(fmtTime(((drag?.times(dur)?.toLong()) ?: pos) / 1000.0), color = Muted, fontSize = 12.sp)
                Text(fmtTime(dur / 1000.0), color = Muted, fontSize = 12.sp)
            }
            Row(Modifier.fillMaxWidth().padding(top = 8.dp, bottom = 16.dp), horizontalArrangement = Arrangement.SpaceEvenly, verticalAlignment = Alignment.CenterVertically) {
                IconButton({ PlayerConn.setShuffle(!st.shuffle) }) { Icon(Icons.Rounded.Shuffle, L("Перемешать", "Shuffle"), tint = if (st.shuffle) Accent else Muted) }
                IconButton({ PlayerConn.prev() }) { Icon(Icons.Rounded.SkipPrevious, L("Назад", "Back"), Modifier.size(36.dp)) }
                Box(Modifier.size(68.dp).clip(CircleShape).background(Accent).clickable { PlayerConn.toggle() }, contentAlignment = Alignment.Center) {
                    if (st.buffering) CircularProgressIndicator(Modifier.size(28.dp), color = Bg, strokeWidth = 3.dp)
                    else Icon(if (st.isPlaying) Icons.Rounded.Pause else Icons.Rounded.PlayArrow, L("Играть/пауза", "Play/pause"), Modifier.size(38.dp), tint = Bg)
                }
                IconButton({ PlayerConn.next() }) { Icon(Icons.Rounded.SkipNext, L("Вперёд", "Next"), Modifier.size(36.dp)) }
                IconButton({ PlayerConn.cycleRepeat() }) {
                    Icon(if (st.repeat == Player.REPEAT_MODE_ONE) Icons.Rounded.RepeatOne else Icons.Rounded.Repeat, L("Повтор", "Repeat"), tint = if (st.repeat != Player.REPEAT_MODE_OFF) Accent else Muted)
                }
            }
        }
    }
}

/**
 * Видео: локальный файл рисуется прямо из основного плеера; клип с YouTube — отдельный беззвучный
 * плеер с видеодорожкой, который подстраивается под позицию музыки (как на ПК).
 */
@OptIn(UnstableApi::class)
@Composable
fun VideoArea(t: Track, playing: Boolean) {
    val ctx = LocalContext.current
    val shape = RoundedCornerShape(14.dp)
    if (t.isLocal) {
        val c = PlayerConn.controller ?: return
        AndroidView(
            factory = { PlayerView(it).apply { useController = false; player = c; resizeMode = AspectRatioFrameLayout.RESIZE_MODE_FIT; setShutterBackgroundColor(android.graphics.Color.BLACK) } },
            modifier = Modifier.fillMaxWidth().aspectRatio(16f / 9f).clip(shape).background(Color.Black),
            onRelease = { it.player = null },
        )
        return
    }
    val vid = t.videoId ?: return
    var url by remember(vid) { mutableStateOf<String?>(null) }
    var err by remember(vid) { mutableStateOf<String?>(null) }
    LaunchedEffect(vid) {
        runCatching { withContext(Dispatchers.IO) { YouTube.videoStream(vid) } }.onSuccess { url = it }.onFailure { err = it.message }
    }
    val u = url
    Box(Modifier.fillMaxWidth().aspectRatio(16f / 9f).clip(shape).background(Color.Black), contentAlignment = Alignment.Center) {
        when {
            err != null -> Text(L("Видео не загрузилось: $err", "The video didn't load: $err"), color = Muted, fontSize = 13.sp, modifier = Modifier.padding(16.dp))
            u == null -> CircularProgressIndicator(color = Accent)
            else -> {
                val vp = remember(u) { ExoPlayer.Builder(ctx).build().apply { volume = 0f; setMediaItem(MediaItem.fromUri(u)); prepare() } }
                DisposableEffect(vp) { onDispose { vp.release() } }
                LaunchedEffect(vp) {
                    // держим видео рядом со звуком: расхождение > 0.4 с — перематываем
                    while (true) {
                        val target = PlayerConn.position()
                        if (kotlin.math.abs(vp.currentPosition - target) > 400) vp.seekTo(target)
                        vp.playWhenReady = PlayerConn.state.value.isPlaying
                        delay(300)
                    }
                }
                AndroidView(
                    factory = { PlayerView(it).apply { useController = false; player = vp; resizeMode = AspectRatioFrameLayout.RESIZE_MODE_FIT; layoutParams = ViewGroup.LayoutParams(-1, -1) } },
                    modifier = Modifier.fillMaxSize(),
                    onRelease = { it.player = null },
                )
            }
        }
    }
}

/** Текст песни: текущая строка подсвечена и прокручивается к середине, тап по строке — перемотка. */
@Composable
fun LyricsView(id: String, posMs: Long) {
    LaunchedEffect(id) { AppState.loadLyrics(id) }
    val st by AppState.lyrics.collectAsState()
    val s = st?.takeIf { it.id == id }
    val text = s?.text
    val list = rememberLazyListState()
    when {
        s == null || (text == null && s.error == null) -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator(color = Accent) }
        s.error != null -> Empty(L("Текст не загрузился", "Lyrics didn't load"), s.error)
        text == null || text.isEmpty -> Empty(L("Текста не нашлось", "No lyrics found"), L("Тексты берутся из открытой базы LRCLIB.", "Lyrics come from the open LRCLIB database."))
        text.instrumental && text.synced.isEmpty() && text.plain.isBlank() -> Empty(L("♪ Инструментал", "♪ Instrumental"), L("Без слов", "No words"))
        text.synced.isNotEmpty() -> {
            val cur = text.lineAt(posMs / 1000.0)
            LaunchedEffect(cur) { if (cur >= 0) list.animateScrollToItem((cur - 3).coerceAtLeast(0)) }
            LazyColumn(Modifier.fillMaxSize().padding(horizontal = 24.dp), state = list) {
                itemsIndexed(text.synced) { i, (t, line) ->
                    Text(line.ifEmpty { "♪" }, Modifier.fillMaxWidth().clickable { PlayerConn.seek((t * 1000).toLong()) }.padding(vertical = 8.dp),
                        fontSize = if (i == cur) 22.sp else 19.sp, fontWeight = if (i == cur) FontWeight.Bold else FontWeight.Medium,
                        color = if (i == cur) TextC else if (i < cur) Muted.copy(alpha = 0.6f) else Muted)
                }
            }
        }
        else -> LazyColumn(Modifier.fillMaxSize().padding(horizontal = 24.dp)) {
            itemsIndexed(text.plain.lines()) { _, line -> Text(line, Modifier.padding(vertical = 4.dp), fontSize = 18.sp, color = TextC) }
        }
    }
}
