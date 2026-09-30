package io.cdus.app.ui.navigation

import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Description
import androidx.compose.material.icons.filled.Devices
import androidx.compose.material.icons.filled.ContentPaste
import androidx.compose.material.icons.filled.Folder
import androidx.compose.material.icons.filled.ListAlt
import androidx.compose.material.icons.filled.Settings
import androidx.compose.ui.graphics.vector.ImageVector

sealed class Screen(val route: String, val title: String, val icon: ImageVector) {
    object Devices : Screen("devices", "Devices", Icons.Default.Devices)
    object Clipboard : Screen("clipboard", "Clipboard", Icons.Default.ContentPaste)
    object Files : Screen("files", "Files", Icons.Default.Folder)
    object Notes : Screen("notes", "Notes", Icons.Default.Description)
    object Audit : Screen("audit", "Audit Log", Icons.Default.ListAlt)
    object Settings : Screen("settings", "Settings", Icons.Default.Settings)
}

val navItems = listOf(
    Screen.Devices,
    Screen.Clipboard,
    Screen.Files,
    Screen.Notes,
    Screen.Audit,
    Screen.Settings
)
