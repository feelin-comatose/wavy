package app.wavy.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.MusicNote
import androidx.compose.material3.Icon
import androidx.compose.material3.LocalContentColor
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.SideEffect
import androidx.compose.ui.platform.LocalView
import androidx.core.view.WindowCompat
import android.app.Activity
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import coil3.compose.SubcomposeAsyncImage

/** Interface theme palette (same themes and colors as the PC version, native/src/theme.rs). */
data class Pal(val dark: Boolean, val bg: Color, val bg2: Color, val bg3: Color, val line: Color, val text: Color, val muted: Color, val accent: Color,
               /** icons and text drawn on an accent fill (play button, primary buttons) */
               val onAccent: Color = if (dark) bg else Color.White)

object Themes {
    private fun pal(dark: Boolean, bg: Long, bg2: Long, bg3: Long, line: Long, text: Long, muted: Long, accent: Long) =
        Pal(dark, Color(0xFF000000 or bg), Color(0xFF000000 or bg2), Color(0xFF000000 or bg3), Color(0xFF000000 or line), Color(0xFF000000 or text), Color(0xFF000000 or muted), Color(0xFF000000 or accent))

    /** (id, Russian name, English name, palette); "dark" is the original black-and-white look */
    val list = listOf(
        Triple("dark", "Тёмная" to "Dark", pal(true, 0x0a0a0c, 0x101013, 0x18181c, 0x232329, 0xe4e4e8, 0x7a7a86, 0xf4f4f5)),
        Triple("amoled", "Чёрная (AMOLED)" to "Black (AMOLED)", pal(true, 0x000000, 0x07070a, 0x101014, 0x1c1c22, 0xe8e8ec, 0x74747e, 0xffffff)),
        Triple("light", "Светлая" to "Light", pal(false, 0xf7f7f8, 0xffffff, 0xececef, 0xdcdce1, 0x16161a, 0x6b6b76, 0x111114)),
        Triple("ocean", "Океан" to "Ocean", pal(true, 0x0a0f1a, 0x0f1624, 0x162033, 0x22304a, 0xe2e8f0, 0x7487a3, 0x60a5fa)),
        Triple("sakura", "Сакура" to "Sakura", pal(true, 0x140b10, 0x1b1016, 0x25161e, 0x37212d, 0xf5e6ee, 0xa07f90, 0xf472b6)),
        Triple("forest", "Лес" to "Forest", pal(true, 0x0a110d, 0x0f1812, 0x15221a, 0x213426, 0xe3efe6, 0x7d9785, 0x4ade80)),
        Triple("amber", "Янтарь" to "Amber", pal(true, 0x120e08, 0x18130b, 0x211a10, 0x33281a, 0xf3ece0, 0x9c8c74, 0xfbbf24)),
        // anime-inspired: sunset sky, neon city, shrine vermilion, mecha purple and lime, pastel
        Triple("twilight", "Сумерки" to "Twilight", pal(true, 0x0f0c1a, 0x15112a, 0x1e1838, 0x2d2550, 0xece6f7, 0x8f84ad, 0xfb923c)),
        Triple("neon", "Неон" to "Neon", pal(true, 0x0b0a14, 0x110f1d, 0x181528, 0x262140, 0xe8eaf7, 0x8682a8, 0x22d3ee)),
        Triple("shrine", "Храм" to "Shrine", pal(true, 0x120a09, 0x190e0c, 0x231412, 0x361f1b, 0xf5e7e3, 0xa08680, 0xf2553c)),
        Triple("mecha", "Меха" to "Mecha", pal(true, 0x150d22, 0x1c1230, 0x261a40, 0x37275a, 0xefe8f8, 0x9585b3, 0xa3e635)),
        Triple("pastel", "Пастель" to "Pastel", pal(false, 0xfbf6fb, 0xffffff, 0xf3e9f4, 0xe6d6e8, 0x241a2b, 0x7c6a86, 0x9333ea)),
    )

    /** Current palette — a Compose state, so the UI recolors when it changes. */
    var current by mutableStateOf(list[0].third)
        private set

    /** Accent swatches offered in settings (besides the theme's own). */
    val accents = listOf("#ef4444", "#f97316", "#f59e0b", "#84cc16", "#22c55e", "#14b8a6", "#0ea5e9", "#3b82f6", "#8b5cf6", "#ec4899")

    /** "#rrggbb" → color, null if malformed. */
    fun parseHex(s: String): Color? = s.trim().removePrefix("#").takeIf { it.length == 6 }?.toLongOrNull(16)?.let { Color(0xFF000000 or it) }

    /** Theme + optional custom accent ("" — the theme's own), like the PC version (native/src/theme.rs resolve). */
    fun set(id: String, accent: String = "") {
        val base = (list.firstOrNull { it.first == id } ?: list[0]).third
        current = parseHex(accent)?.let { a -> base.copy(accent = a, onAccent = if (a.luminance() > 0.45f) Color(0xFF0A0A0C) else Color.White) } ?: base
    }
}

// Palette colors used across the UI; they follow the current theme.
val Bg: Color get() = Themes.current.bg
val Bg2: Color get() = Themes.current.bg2
val Bg3: Color get() = Themes.current.bg3
val Line: Color get() = Themes.current.line
val TextC: Color get() = Themes.current.text
val Muted: Color get() = Themes.current.muted
val Accent: Color get() = Themes.current.accent
val OnAccent: Color get() = Themes.current.onAccent
val Bad = Color(0xFFF87171)
val YtRed = Color(0xFFFF0033)

@Composable
fun WavyTheme(content: @Composable () -> Unit) {
    val dark = Themes.current.dark
    val base = if (dark) darkColorScheme() else lightColorScheme()
    // status/navigation bar icons follow the app theme, not the system's dark mode
    val view = LocalView.current
    if (!view.isInEditMode) SideEffect {
        (view.context as? Activity)?.window?.let { w ->
            WindowCompat.getInsetsController(w, view).apply { isAppearanceLightStatusBars = !dark; isAppearanceLightNavigationBars = !dark }
        }
    }
    MaterialTheme(
        colorScheme = base.copy(
            primary = Accent, onPrimary = OnAccent, secondary = Muted, onSecondary = Bg,
            background = Bg, onBackground = TextC, surface = Bg2, onSurface = TextC,
            surfaceVariant = Bg3, onSurfaceVariant = Muted, outline = Line, outlineVariant = Line,
            surfaceContainer = Bg2, surfaceContainerHigh = Bg3, surfaceContainerHighest = Bg3, surfaceContainerLow = Bg2,
            secondaryContainer = Bg3, onSecondaryContainer = TextC, error = Bad,
        ),
    ) {
        // default text color is light everywhere (not only inside Scaffold): otherwise it's black text on black
        CompositionLocalProvider(LocalContentColor provides TextC, content = content)
    }
}

/** Cover with a placeholder note and a red badge on YouTube tracks. */
@Composable
fun Cover(url: String?, size: Dp, radius: Dp = 8.dp, youtube: Boolean = false, modifier: Modifier = Modifier) {
    Box(modifier.size(size)) {
        val shape = RoundedCornerShape(radius)
        val placeholder = @Composable {
            Box(Modifier.size(size).clip(shape).background(Bg3), contentAlignment = Alignment.Center) {
                Icon(Icons.Rounded.MusicNote, null, tint = Muted, modifier = Modifier.size(size * 0.45f))
            }
        }
        if (url == null) placeholder() else SubcomposeAsyncImage(
            model = url, contentDescription = null, contentScale = ContentScale.Crop,
            modifier = Modifier.size(size).clip(shape), loading = { placeholder() }, error = { placeholder() },
        )
        if (youtube) Box(Modifier.align(Alignment.BottomEnd).size(size * 0.22f).clip(RoundedCornerShape(3.dp)).background(YtRed))
    }
}
