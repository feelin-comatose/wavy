package app.wavy.android.ui

import app.wavy.android.data.L
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.VolumeUp
import androidx.compose.material.icons.rounded.Pause
import androidx.compose.material.icons.rounded.PlayArrow
import androidx.compose.material.icons.rounded.SkipNext
import androidx.compose.material.icons.rounded.SkipPrevious
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.wavy.android.data.fmtTime
import app.wavy.android.data.json
import app.wavy.android.net.Http
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.Serializable
import okhttp3.MediaType.Companion.toMediaTypeOrNull
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody

/** Remote: the phone controls Wavy on the PC over Wi-Fi (address and PIN are the same as for sync). */
object Remote {
    @Serializable data class Track(val id: String = "", val title: String = "", val artist: String = "", val cover: String? = null)
    @Serializable data class Item(val i: Int = 0, val title: String = "", val artist: String = "", val current: Boolean = false)
    @Serializable data class State(
        val track: Track? = null, val playing: Boolean = false, val position: Double = 0.0, val duration: Double = 0.0,
        val volume: Double = 0.8, val queue: List<Item> = emptyList(),
    )

    fun base(addr: String) = "http://" + addr.trim().removePrefix("http://").trimEnd('/').let { if (':' in it) it else "$it:3738" }

    fun parse(text: String): State = json.decodeFromString(State.serializer(), text)

    fun get(addr: String, pin: String): State = parse(Http.get("${base(addr)}/wavy/remote?pin=${pin.trim()}"))

    fun cmd(addr: String, pin: String, cmd: String, v: Double = 0.0): State {
        val req = Request.Builder().url("${base(addr)}/wavy/remote?pin=${pin.trim()}&cmd=$cmd&v=$v").post("".toRequestBody("text/plain".toMediaTypeOrNull())).build()
        Http.client.newCall(req).execute().use { r ->
            if (r.code == 403) throw java.io.IOException(L("неверный PIN", "wrong PIN"))
            if (!r.isSuccessful) throw java.io.IOException(L("ПК ответил ${r.code}", "The PC replied ${r.code}"))
            return parse(r.body?.string() ?: "{}")
        }
    }
}

@Composable
fun RemoteScreen(addr: String, pin: String, back: () -> Unit, initial: Remote.State? = null) {
    var st by remember { mutableStateOf(initial) }
    var err by remember { mutableStateOf<String?>(null) }
    var seeking by remember { mutableStateOf<Float?>(null) }
    var vol by remember { mutableStateOf<Float?>(null) }
    val scope = rememberCoroutineScope()
    fun send(c: String, v: Double = 0.0) = scope.launch {
        try { st = withContext(Dispatchers.IO) { Remote.cmd(addr, pin, c, v) }; err = null } catch (e: Exception) { err = e.message ?: L("ПК не отвечает", "The PC isn't responding") }
    }
    // once a second, ask the PC what's playing
    if (initial == null) LaunchedEffect(addr, pin) {
        while (true) {
            try { st = withContext(Dispatchers.IO) { Remote.get(addr, pin) }; err = null } catch (e: Exception) { err = L("ПК не отвечает: ${e.message ?: ""}. Включена ли синхронизация по Wi-Fi на ПК, и в одной ли вы сети?", "The PC isn't responding: ${e.message ?: ""}. Is Wi-Fi sync on in the PC app, and are you on the same network?") }
            delay(1000)
        }
    }
    Column(Modifier.fillMaxSize().background(Bg)) {
        Header(L("Пульт ПК", "PC remote"), addr, back = back)
        err?.let { Text(it, Modifier.padding(horizontal = 16.dp), color = Bad, fontSize = 13.sp) }
        val s = st ?: return@Column
        val t = s.track
        Column(Modifier.fillMaxWidth().padding(16.dp), horizontalAlignment = Alignment.CenterHorizontally) {
            val cover = t?.cover?.let { if (it == "local") "${Remote.base(addr)}/wavy/remote/cover?pin=$pin&id=${t.id}" else it }
            Cover(cover, 220.dp, radius = 16.dp)
            Spacer(Modifier.height(14.dp))
            Text(t?.title ?: L("Ничего не играет", "Nothing is playing"), fontWeight = FontWeight.Bold, fontSize = 20.sp, maxLines = 2, overflow = TextOverflow.Ellipsis)
            if (!t?.artist.isNullOrEmpty()) Text(t!!.artist, color = Muted, fontSize = 15.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Spacer(Modifier.height(8.dp))
            val dur = s.duration.toFloat().coerceAtLeast(1f)
            Slider(seeking ?: s.position.toFloat().coerceIn(0f, dur), { seeking = it }, Modifier.fillMaxWidth(), valueRange = 0f..dur,
                onValueChangeFinished = { seeking?.let { send("seek", it.toDouble()) }; seeking = null },
                colors = SliderDefaults.colors(thumbColor = Accent, activeTrackColor = Accent, inactiveTrackColor = Bg3))
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                Text(fmtTime((seeking ?: s.position.toFloat()).toDouble()), color = Muted, fontSize = 12.sp)
                Text(fmtTime(s.duration), color = Muted, fontSize = 12.sp)
            }
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(18.dp)) {
                IconButton({ send("prev") }) { Icon(Icons.Rounded.SkipPrevious, L("Предыдущий", "Previous"), Modifier.size(34.dp)) }
                Box(Modifier.size(68.dp).background(Accent, CircleShape).clickable { send("toggle") }, contentAlignment = Alignment.Center) {
                    Icon(if (s.playing) Icons.Rounded.Pause else Icons.Rounded.PlayArrow, if (s.playing) L("Пауза", "Pause") else L("Играть", "Play"), tint = OnAccent, modifier = Modifier.size(36.dp))
                }
                IconButton({ send("next") }) { Icon(Icons.Rounded.SkipNext, L("Следующий", "Next"), Modifier.size(34.dp)) }
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(Icons.AutoMirrored.Rounded.VolumeUp, L("Громкость", "Volume"), tint = Muted)
                Slider(vol ?: s.volume.toFloat(), { vol = it }, Modifier.weight(1f).padding(start = 8.dp),
                    onValueChangeFinished = { vol?.let { send("volume", it.toDouble()) }; vol = null },
                    colors = SliderDefaults.colors(thumbColor = Accent, activeTrackColor = Accent, inactiveTrackColor = Bg3))
            }
        }
        if (s.queue.isNotEmpty()) Text(L("Очередь", "Queue"), Modifier.padding(horizontal = 16.dp), fontWeight = FontWeight.SemiBold)
        LazyColumn(Modifier.fillMaxWidth()) {
            items(s.queue, key = { it.i }) { q ->
                Row(Modifier.fillMaxWidth().clickable { send("jump", q.i.toDouble()) }.padding(horizontal = 16.dp, vertical = 9.dp)) {
                    Column(Modifier.weight(1f)) {
                        Text(q.title, color = if (q.current) Accent else TextC, fontWeight = if (q.current) FontWeight.Bold else FontWeight.Medium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        if (q.artist.isNotEmpty()) Text(q.artist, color = Muted, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                }
            }
        }
    }
}
