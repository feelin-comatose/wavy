package app.wavy.android

import android.app.Application
import app.wavy.android.data.Store
import app.wavy.android.yt.YouTube

class WavyApp : Application() {
    override fun onCreate() {
        super.onCreate()
        Store.init(this)
        app.wavy.android.data.Offline.init(filesDir)
        app.wavy.android.data.Lyrics.init(filesDir)
        Thread { runCatching { YouTube.init() } }.start()
    }
}
