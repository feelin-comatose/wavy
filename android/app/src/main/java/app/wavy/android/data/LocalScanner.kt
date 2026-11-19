package app.wavy.android.data

import android.content.ContentUris
import android.content.Context
import android.net.Uri
import android.provider.MediaStore

/** Музыка и видео из памяти телефона (MediaStore). */
object LocalScanner {
    fun scan(ctx: Context): List<Track> {
        val out = ArrayList<Track>()
        val now = System.currentTimeMillis()
        val cr = ctx.contentResolver
        // музыка
        val aCols = arrayOf(
            MediaStore.Audio.Media._ID, MediaStore.Audio.Media.TITLE, MediaStore.Audio.Media.ARTIST,
            MediaStore.Audio.Media.ALBUM, MediaStore.Audio.Media.ALBUM_ID, MediaStore.Audio.Media.DURATION, MediaStore.Audio.Media.DATE_ADDED,
        )
        cr.query(MediaStore.Audio.Media.EXTERNAL_CONTENT_URI, aCols, "${MediaStore.Audio.Media.IS_MUSIC} != 0", null, null)?.use { c ->
            while (c.moveToNext()) {
                val id = c.getLong(0)
                val uri = ContentUris.withAppendedId(MediaStore.Audio.Media.EXTERNAL_CONTENT_URI, id)
                val art = ContentUris.withAppendedId(Uri.parse("content://media/external/audio/albumart"), c.getLong(4))
                out += Track(
                    id = "and_a_$id", type = "local", path = uri.toString(),
                    title = c.getString(1) ?: "Без названия",
                    artist = c.getString(2)?.takeIf { it != "<unknown>" } ?: "Неизвестный исполнитель",
                    album = c.getString(3) ?: "", duration = c.getLong(5) / 1000.0,
                    cover = art.toString(), addedAt = c.getLong(6) * 1000,
                )
            }
        }
        // видео
        val vCols = arrayOf(MediaStore.Video.Media._ID, MediaStore.Video.Media.TITLE, MediaStore.Video.Media.DURATION, MediaStore.Video.Media.DATE_ADDED)
        cr.query(MediaStore.Video.Media.EXTERNAL_CONTENT_URI, vCols, null, null, null)?.use { c ->
            while (c.moveToNext()) {
                val id = c.getLong(0)
                val uri = ContentUris.withAppendedId(MediaStore.Video.Media.EXTERNAL_CONTENT_URI, id)
                out += Track(
                    id = "and_v_$id", type = "local", path = uri.toString(), title = c.getString(1) ?: "Видео",
                    artist = "Видео", duration = c.getLong(2) / 1000.0, cover = uri.toString(),
                    addedAt = c.getLong(3) * 1000, video = true,
                )
            }
        }
        if (out.isEmpty()) return out
        return out.map { if (it.addedAt == 0L) it.copy(addedAt = now) else it }
    }
}
