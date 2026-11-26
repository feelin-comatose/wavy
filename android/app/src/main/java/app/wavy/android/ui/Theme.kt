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
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import coil3.compose.SubcomposeAsyncImage

// the same monochrome palette as on the PC
val Bg = Color(0xFF0A0A0C)
val Bg2 = Color(0xFF101013)
val Bg3 = Color(0xFF18181C)
val Line = Color(0xFF232329)
val TextC = Color(0xFFE4E4E8)
val Muted = Color(0xFF7A7A86)
val Accent = Color(0xFFF4F4F5)
val Bad = Color(0xFFF87171)
val YtRed = Color(0xFFFF0033)

@Composable
fun WavyTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = darkColorScheme(
            primary = Accent, onPrimary = Bg, secondary = Muted, onSecondary = Bg,
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
