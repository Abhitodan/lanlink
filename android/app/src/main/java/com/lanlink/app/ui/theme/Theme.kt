package com.lanlink.app.ui.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

// Calm, content-first palette: ivory surfaces, graphite text, a muted blue
// for actions and a warm sandstone accent used sparingly.
object Palette {
    val Ivory = Color(0xFFFAF8F3)
    val IvoryDeep = Color(0xFFF1EDE4)
    val Line = Color(0xFFE2DDD2)
    val Graphite = Color(0xFF1F2328)
    val GraphiteSoft = Color(0xFF5B616B)
    val Blue = Color(0xFF2F4A6D)
    val BlueSoft = Color(0xFFDCE4EE)
    val Sandstone = Color(0xFFD9B98C)
    val SandstoneSoft = Color(0xFFF3E7D5)
    val Danger = Color(0xFFA23B33)
    val Ok = Color(0xFF3E7A55)

    val Night = Color(0xFF15181C)
    val NightRaised = Color(0xFF1E2227)
    val NightLine = Color(0xFF2D3239)
    val Mist = Color(0xFFE9E6DF)
    val MistSoft = Color(0xFFA9AEB6)
    val BlueNight = Color(0xFF9DB6D6)
}

private val Light = lightColorScheme(
    primary = Palette.Blue,
    onPrimary = Palette.Ivory,
    primaryContainer = Palette.BlueSoft,
    onPrimaryContainer = Palette.Blue,
    secondary = Palette.Sandstone,
    onSecondary = Palette.Graphite,
    secondaryContainer = Palette.SandstoneSoft,
    onSecondaryContainer = Palette.Graphite,
    background = Palette.Ivory,
    onBackground = Palette.Graphite,
    surface = Palette.Ivory,
    onSurface = Palette.Graphite,
    surfaceVariant = Palette.IvoryDeep,
    onSurfaceVariant = Palette.GraphiteSoft,
    outline = Palette.Line,
    outlineVariant = Palette.Line,
    error = Palette.Danger,
)

private val Dark = darkColorScheme(
    primary = Palette.BlueNight,
    onPrimary = Palette.Night,
    primaryContainer = Color(0xFF2A3B52),
    onPrimaryContainer = Palette.Mist,
    secondary = Palette.Sandstone,
    onSecondary = Palette.Night,
    secondaryContainer = Color(0xFF3A3226),
    onSecondaryContainer = Palette.Mist,
    background = Palette.Night,
    onBackground = Palette.Mist,
    surface = Palette.Night,
    onSurface = Palette.Mist,
    surfaceVariant = Palette.NightRaised,
    onSurfaceVariant = Palette.MistSoft,
    outline = Palette.NightLine,
    outlineVariant = Palette.NightLine,
    error = Color(0xFFE08A80),
)

private val Type = Typography(
    headlineMedium = TextStyle(fontFamily = FontFamily.SansSerif, fontWeight = FontWeight.SemiBold, fontSize = 26.sp, lineHeight = 32.sp, letterSpacing = (-0.2).sp),
    titleLarge = TextStyle(fontFamily = FontFamily.SansSerif, fontWeight = FontWeight.SemiBold, fontSize = 20.sp, lineHeight = 26.sp),
    titleMedium = TextStyle(fontFamily = FontFamily.SansSerif, fontWeight = FontWeight.Medium, fontSize = 16.sp, lineHeight = 22.sp),
    bodyLarge = TextStyle(fontFamily = FontFamily.SansSerif, fontSize = 16.sp, lineHeight = 24.sp),
    bodyMedium = TextStyle(fontFamily = FontFamily.SansSerif, fontSize = 14.sp, lineHeight = 20.sp),
    labelLarge = TextStyle(fontFamily = FontFamily.SansSerif, fontWeight = FontWeight.Medium, fontSize = 14.sp, letterSpacing = 0.1.sp),
    labelMedium = TextStyle(fontFamily = FontFamily.SansSerif, fontWeight = FontWeight.Medium, fontSize = 12.sp, letterSpacing = 0.3.sp),
)

private val AppShapes = Shapes(
    small = RoundedCornerShape(8.dp),
    medium = RoundedCornerShape(12.dp),
    large = RoundedCornerShape(16.dp),
)

@Composable
fun LanLinkTheme(dark: Boolean = isSystemInDarkTheme(), content: @Composable () -> Unit) {
    MaterialTheme(colorScheme = if (dark) Dark else Light, typography = Type, shapes = AppShapes, content = content)
}
