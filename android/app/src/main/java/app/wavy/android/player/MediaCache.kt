package app.wavy.android.player

import android.content.Context
import android.net.Uri
import androidx.media3.common.util.UnstableApi
import androidx.media3.database.StandaloneDatabaseProvider
import androidx.media3.datasource.DataSource
import androidx.media3.datasource.DataSpec
import androidx.media3.datasource.TransferListener
import androidx.media3.datasource.cache.CacheDataSource
import androidx.media3.datasource.cache.LeastRecentlyUsedCacheEvictor
import androidx.media3.datasource.cache.SimpleCache
import java.io.File

/**
 * Disk cache for YouTube tracks (512 MB, oldest evicted): replayed tracks start instantly.
 * The cache key is the wavy://yt/<id> address, not the temporary googlevideo link, so the cache survives the link expiring.
 * Local files aren't cached — they're already on the phone.
 */
@UnstableApi
object MediaCache {
    @Volatile private var cache: SimpleCache? = null

    fun get(ctx: Context): SimpleCache = cache ?: synchronized(this) {
        cache ?: SimpleCache(File(ctx.cacheDir, "media"), LeastRecentlyUsedCacheEvictor(512L * 1024 * 1024), StandaloneDatabaseProvider(ctx)).also { cache = it }
    }

    /** wavy://… goes through the cache, everything else directly. */
    class Factory(ctx: Context, private val direct: DataSource.Factory, upstream: DataSource.Factory) : DataSource.Factory {
        private val cached = CacheDataSource.Factory().setCache(get(ctx)).setUpstreamDataSourceFactory(upstream)
            .setFlags(CacheDataSource.FLAG_IGNORE_CACHE_ON_ERROR)
        override fun createDataSource(): DataSource = Switch(cached.createDataSource(), direct.createDataSource())
    }

    private class Switch(private val cached: DataSource, private val direct: DataSource) : DataSource {
        private var cur: DataSource? = null
        override fun addTransferListener(l: TransferListener) { cached.addTransferListener(l); direct.addTransferListener(l) }
        override fun open(spec: DataSpec): Long = (if (spec.uri.scheme == "wavy") cached else direct).also { cur = it }.open(spec)
        override fun read(buffer: ByteArray, offset: Int, length: Int) = cur!!.read(buffer, offset, length)
        override fun getUri(): Uri? = cur?.uri
        override fun getResponseHeaders(): Map<String, List<String>> = cur?.responseHeaders ?: emptyMap()
        override fun close() { cur?.close(); cur = null }
    }
}
