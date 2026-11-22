package app.wavy.android.data

import app.wavy.android.net.Http
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.put
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.UUID
import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.ln
import kotlin.math.max
import kotlin.math.sin

/**
 * Распознавание музыки через Shazam: из 12 секунд звука (моно, 16 кГц) строится «отпечаток», он уходит
 * на сервер Shazam. Алгоритм — из SongRec (GPL-3.0, как и Wavy), тот же, что в версии для ПК
 * (core/src/shazam.rs): отпечатки обеих версий совпадают побайтово (см. LogicTest).
 */
object Shazam {
    const val RATE = 16_000
    const val SECONDS = 12

    data class Found(val title: String, val artist: String, val cover: String?, val url: String?)

    private class Peak(val pass: Int, val magnitude: Int, val bin: Int)

    /** Отпечаток из моно-звука 16 кГц (значения −1..1). */
    fun signature(samples: FloatArray): ByteArray {
        val g = Gen()
        val s16 = ShortArray(samples.size) { (samples[it] * 32768f).coerceIn(-32768f, 32767f).toInt().toShort() }
        var i = 0
        while (i + 128 <= s16.size) {
            g.fft(s16, i)
            g.spread()
            g.done++
            if (g.done >= 46) g.recognize()
            i += 128
        }
        return encode(g.peaks, samples.size)
    }

    private class Gen {
        val ring = ShortArray(2048)
        var ringI = 0
        val window = FloatArray(2048) { (0.5 * (1.0 - cos(2.0 * PI * (it + 1) / 2049.0))).toFloat() }
        val fft = Fft(2048)
        val outs = Array(256) { FloatArray(1025) }
        var outsI = 0
        val spreadBuf = Array(256) { FloatArray(1025) }
        var spreadI = 0
        var done = 0
        val peaks = Array(4) { ArrayList<Peak>() }
        private val re = FloatArray(2048)
        private val im = FloatArray(2048)

        fun fft(chunk: ShortArray, off: Int) {
            System.arraycopy(chunk, off, ring, ringI, 128)
            ringI = (ringI + 128) and 2047
            for (i in 0 until 2048) { re[i] = ring[(i + ringI) and 2047] * window[i]; im[i] = 0f }
            fft.run(re, im)
            val out = outs[outsI]
            for (i in 0..1024) out[i] = max((re[i] * re[i] + im[i] * im[i]) / (1 shl 17).toFloat(), 1e-10f)
            outsI = (outsI + 1) and 255
        }

        fun spread() {
            val cur = outs[(outsI - 1) and 255].copyOf()
            for (p in 0..1022) cur[p] = max(max(cur[p], cur[p + 1]), cur[p + 2])
            spreadBuf[spreadI] = cur
            for (p in 0..1024) for (back in intArrayOf(1, 3, 6)) {
                val f = spreadBuf[(spreadI - back) and 255]
                f[p] = max(f[p], cur[p])
            }
            spreadI = (spreadI + 1) and 255
        }

        fun recognize() {
            val f46 = outs[(outsI - 46) and 255]
            val f49 = spreadBuf[(spreadI - 49) and 255]
            for (bin in 10..1014) {
                if (!(f46[bin] >= 1f / 64f && f46[bin] >= f49[bin - 1])) continue
                var maxN = 0f
                for (off in intArrayOf(-10, -7, -4, -3, 1, 2, 5, 8)) maxN = max(maxN, f49[bin + off])
                if (f46[bin] <= maxN) continue
                var maxO = maxN
                for (off in intArrayOf(-53, -45, 165, 172, 179, 186, 193, 200, 214, 221, 228, 235, 242, 249)) {
                    maxO = max(maxO, spreadBuf[(spreadI + off) and 255][bin - 1])
                }
                if (f46[bin] <= maxO) continue
                fun mag(v: Float) = max(ln(v), 1f / 64f) * 1477.3f + 6144f
                val m = mag(f46[bin]); val before = mag(f46[bin - 1]); val after = mag(f46[bin + 1])
                val var1 = m * 2f - before - after
                if (var1 <= 0f) continue
                val var2 = (after - before) * 32f / var1
                val corrected = (bin * 64 + var2.toInt()) and 0xffff
                val hz = corrected * (16000f / 2f / 1024f / 64f)
                val band = when (hz.toInt()) { in 250..519 -> 0; in 520..1449 -> 1; in 1450..3499 -> 2; in 3500..5500 -> 3; else -> continue }
                peaks[band].add(Peak(done - 46, m.toInt() and 0xffff, corrected))
            }
        }
    }

    private fun encode(peaks: Array<ArrayList<Peak>>, samples: Int): ByteArray {
        val b = java.io.ByteArrayOutputStream()
        fun w(v: Int) { b.write(ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN).putInt(v).array()) }
        w(0xcafe2580.toInt()); w(0); w(0); w(0x94119c00.toInt())
        repeat(3) { w(0) }
        w(3 shl 27)
        repeat(2) { w(0) }
        w(samples + (16000.0 * 0.24).toInt())
        w((15 shl 19) + 0x40000)
        w(0x40000000)
        w(0)
        for ((band, list) in peaks.withIndex()) {
            if (list.isEmpty()) continue
            val pb = java.io.ByteArrayOutputStream()
            var pass = 0
            for (p in list) {
                if (p.pass - pass >= 255) {
                    pb.write(0xff)
                    pb.write(ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN).putInt(p.pass).array())
                    pass = p.pass
                }
                pb.write(p.pass - pass)
                pb.write(ByteBuffer.allocate(2).order(ByteOrder.LITTLE_ENDIAN).putShort(p.magnitude.toShort()).array())
                pb.write(ByteBuffer.allocate(2).order(ByteOrder.LITTLE_ENDIAN).putShort(p.bin.toShort()).array())
                pass = p.pass
            }
            w(0x60030040 + band)
            w(pb.size())
            val pad = (4 - pb.size() % 4) % 4
            b.write(pb.toByteArray())
            repeat(pad) { b.write(0) }
        }
        val out = b.toByteArray()
        val size = out.size - 48
        val sz = ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN).putInt(size).array()
        System.arraycopy(sz, 0, out, 8, 4)
        System.arraycopy(sz, 0, out, 52, 4)
        val crc = java.util.zip.CRC32().apply { update(out, 8, out.size - 8) }.value.toInt()
        System.arraycopy(ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN).putInt(crc).array(), 0, out, 4, 4)
        return out
    }

    private class Fft(val n: Int) {
        private val bits = Integer.numberOfTrailingZeros(n)
        private val rev = IntArray(n) { Integer.reverse(it) ushr (32 - bits) }
        private val cs = FloatArray(n / 2) { cos(-2.0 * PI * it / n).toFloat() }
        private val sn = FloatArray(n / 2) { sin(-2.0 * PI * it / n).toFloat() }
        fun run(re: FloatArray, im: FloatArray) {
            for (i in 0 until n) { val j = rev[i]; if (j > i) { var t = re[i]; re[i] = re[j]; re[j] = t; t = im[i]; im[i] = im[j]; im[j] = t } }
            var len = 2
            while (len <= n) {
                val step = n / len
                var start = 0
                while (start < n) {
                    for (k in 0 until len / 2) {
                        val c = cs[k * step]; val s = sn[k * step]
                        val a = start + k; val bb = start + k + len / 2
                        val tr = re[bb] * c - im[bb] * s; val ti = re[bb] * s + im[bb] * c
                        re[bb] = re[a] - tr; im[bb] = im[a] - ti
                        re[a] += tr; im[a] += ti
                    }
                    start += len
                }
                len = len shl 1
            }
        }
    }

    /** Отправить 12 с звука (моно 16 кГц) в Shazam. null — не узнал. */
    fun recognize(samples: FloatArray): Found? {
        val sig = signature(samples)
        val now = System.currentTimeMillis()
        val body = buildJsonObject {
            put("geolocation", buildJsonObject { put("altitude", 300); put("latitude", 45); put("longitude", 2) })
            put("signature", buildJsonObject {
                put("samplems", (samples.size.toDouble() / RATE * 1000).toLong())
                put("timestamp", now)
                put("uri", "data:audio/vnd.shazam.sig;base64," + java.util.Base64.getEncoder().encodeToString(sig))
            })
            put("timestamp", now)
            put("timezone", "Europe/Moscow")
        }.toString()
        val url = "https://amp.shazam.com/discovery/v5/en/US/android/-/tag/${UUID.randomUUID().toString().uppercase()}/${UUID.randomUUID()}" +
            "?sync=true&webv3=true&sampling=true&connected=&shazamapiversion=v3&sharehub=true&video=v3"
        val req = Request.Builder().url(url)
            .header("User-Agent", "Dalvik/2.1.0 (Linux; U; Android 10; SM-G973F Build/QP1A.190711.020)")
            .header("Content-Language", "en_US")
            .post(body.toRequestBody("application/json".toMediaType())).build()
        Http.client.newCall(req).execute().use { r ->
            if (r.code == 429) throw java.io.IOException(L("Shazam: слишком много запросов, попробуй через минуту", "Shazam: too many requests, try again in a minute"))
            if (!r.isSuccessful) throw java.io.IOException(L("Shazam ответил ${r.code}", "Shazam replied ${r.code}"))
            return parse(r.body?.string() ?: "")
        }
    }

    fun parse(text: String): Found? {
        val v = runCatching { json.parseToJsonElement(text).jsonObject }.getOrNull() ?: return null
        val t = v["track"] as? JsonObject ?: return null
        fun s(o: JsonObject?, k: String) = (o?.get(k) as? JsonPrimitive)?.contentOrNull
        val title = s(t, "title") ?: return null
        val img = t["images"] as? JsonObject
        return Found(title, s(t, "subtitle") ?: "", s(img, "coverarthq") ?: s(img, "coverart"), s(t, "url"))
    }
}
