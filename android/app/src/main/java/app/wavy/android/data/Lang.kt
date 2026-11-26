package app.wavy.android.data

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue

/** Interface language: Russian (default) or English. Translations live in the code: L("Настройки", "Settings"). */
object Lang {
    var en by mutableStateOf(false)
    fun set(code: String) { en = code == "en" }
}

/** String in the current language (Compose redraws the screen when the language changes). */
fun L(ru: String, en: String): String = if (Lang.en) en else ru
