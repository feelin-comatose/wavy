package app.wavy.android

import android.app.Application
import app.wavy.android.data.Store
import app.wavy.android.yt.YouTube

class WavyApp : Application() {
    override fun onCreate() {
        super.onCreate()
        Store.init(this)
        Thread { runCatching { YouTube.init() } }.start()
    }
}
