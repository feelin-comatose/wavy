package app.wavy.android.data

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue

/** Язык интерфейса: русский (по умолчанию) или английский. Переводы — прямо в коде: L("Настройки", "Settings"). */
object Lang {
    var en by mutableStateOf(false)
    fun set(code: String) { en = code == "en" }
}

/** Строка на текущем языке (Compose перерисует экран при смене языка). */
fun L(ru: String, en: String): String = if (Lang.en) en else ru
