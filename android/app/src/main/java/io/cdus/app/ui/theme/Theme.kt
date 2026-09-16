package io.cdus.app.ui.theme

import android.os.Build
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext

private val DarkColorScheme = darkColorScheme(
    primary = CdusCyan,
    onPrimary = Color(0xFF00363A),
    primaryContainer = CdusCyanContainerDark,
    onPrimaryContainer = CdusCyanLight,
    secondary = CdusStatusOnline,
    onSecondary = Color.White,
    secondaryContainer = CdusLanBgDark,
    onSecondaryContainer = CdusLanTextDark,
    tertiary = CdusStatusRelay,
    onTertiary = Color.White,
    tertiaryContainer = CdusRelayBgDark,
    onTertiaryContainer = CdusRelayTextDark,
    error = CdusStatusOffline,
    onError = Color.White,
    errorContainer = CdusErrorBgDark,
    onErrorContainer = CdusErrorTextDark,
    background = CdusDarkBase,
    onBackground = CdusTextPrimaryDark,
    surface = CdusDarkSurface,
    onSurface = CdusTextPrimaryDark,
    surfaceVariant = CdusDarkSurfaceVariant,
    onSurfaceVariant = CdusTextSecondaryDark,
    outline = CdusDarkBorder,
    outlineVariant = CdusDarkBorderSubtle
)

private val LightColorScheme = lightColorScheme(
    primary = CdusCyanDark,
    onPrimary = Color.White,
    primaryContainer = CdusCyanContainerLight,
    onPrimaryContainer = CdusCyanDark,
    secondary = CdusLanTextLight,
    onSecondary = Color.White,
    secondaryContainer = CdusLanBgLight,
    onSecondaryContainer = CdusLanTextLight,
    tertiary = CdusRelayTextLight,
    onTertiary = Color.White,
    tertiaryContainer = CdusRelayBgLight,
    onTertiaryContainer = CdusRelayTextLight,
    error = CdusErrorText,
    onError = Color.White,
    errorContainer = CdusErrorBgLight,
    onErrorContainer = CdusErrorText,
    background = CdusLightBase,
    onBackground = CdusTextPrimaryLight,
    surface = CdusLightCard,
    onSurface = CdusTextPrimaryLight,
    surfaceVariant = CdusLightSurface,
    onSurfaceVariant = CdusTextSecondaryLight,
    outline = CdusLightBorder,
    outlineVariant = CdusLightBorderSubtle
)

@Composable
fun CdusandroidTheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    // Default to false to preserve CDUS signature identity over generic wallpaper dynamic color
    dynamicColor: Boolean = false,
    content: @Composable () -> Unit
) {
    val colorScheme = when {
        dynamicColor && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S -> {
            val context = LocalContext.current
            if (darkTheme) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
        }
        darkTheme -> DarkColorScheme
        else -> LightColorScheme
    }

    MaterialTheme(
        colorScheme = colorScheme,
        typography = Typography,
        content = content
    )
}