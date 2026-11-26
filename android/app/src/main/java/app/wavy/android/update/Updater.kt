package app.wavy.android.update

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.core.content.FileProvider
import app.wavy.android.BuildConfig
import app.wavy.android.data.json
import app.wavy.android.net.Http
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import okhttp3.Request
import java.io.File

/** Auto-update: GitHub Releases (build-N builds; N is baked into the APK as versionCode). */
object Updater {
    const val REPO = "feelin-comatose/wavy"
    data class Release(val build: Int, val url: String)

    val currentBuild get() = BuildConfig.WAVY_BUILD

    fun check(): Release? {
        val v = json.parseToJsonElement(Http.get("https://api.github.com/repos/$REPO/releases/latest", mapOf("Accept" to "application/vnd.github+json"))).jsonObject
        val build = v["tag_name"]?.jsonPrimitive?.content?.removePrefix("build-")?.toIntOrNull() ?: return null
        if (build <= currentBuild) return null
        val apk = v["assets"]?.jsonArray?.map { it.jsonObject }?.firstOrNull { (it["name"]?.jsonPrimitive?.content ?: "").endsWith(".apk") } ?: return null
        return Release(build, apk["browser_download_url"]!!.jsonPrimitive.content)
    }

    fun download(ctx: Context, rel: Release, progress: (Int) -> Unit): File {
        val dir = File(ctx.cacheDir, "updates").apply { mkdirs(); listFiles()?.forEach { it.delete() } }
        val out = File(dir, "Wavy-${rel.build}.apk")
        Http.client.newCall(Request.Builder().url(rel.url).header("User-Agent", Http.UA).build()).execute().use { r ->
            if (!r.isSuccessful) throw java.io.IOException("HTTP ${r.code}")
            val body = r.body!!
            val total = body.contentLength()
            body.byteStream().use { input -> out.outputStream().use { o ->
                val buf = ByteArray(64 * 1024); var done = 0L; var n: Int
                while (input.read(buf).also { n = it } >= 0) { o.write(buf, 0, n); done += n; if (total > 0) progress((done * 100 / total).toInt()) }
            } }
        }
        return out
    }

    /** Whether the app is allowed to install APKs (Android asks once). */
    fun canInstall(ctx: Context) = Build.VERSION.SDK_INT < 26 || ctx.packageManager.canRequestPackageInstalls()

    fun openInstallPermission(ctx: Context) {
        ctx.startActivity(Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:${ctx.packageName}")).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    }

    fun install(ctx: Context, apk: File) {
        val uri = FileProvider.getUriForFile(ctx, "${ctx.packageName}.files", apk)
        ctx.startActivity(Intent(Intent.ACTION_VIEW).setDataAndType(uri, "application/vnd.android.package-archive")
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK))
    }
}
