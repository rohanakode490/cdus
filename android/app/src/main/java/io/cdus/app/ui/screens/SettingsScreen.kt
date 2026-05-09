package io.cdus.app.ui.screens

import android.content.Intent
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import io.cdus.app.service.SyncService

@Composable
fun SettingsScreen() {
    val context = LocalContext.current
    var isSyncEnabled by remember { mutableStateOf(false) }
    var deviceName by remember { mutableStateOf("Pixel 7") }
    var clipboardLimit by remember { mutableFloatStateOf(50f) }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp)
    ) {
        Text(
            text = "Settings",
            style = MaterialTheme.typography.headlineMedium,
            modifier = Modifier.padding(bottom = 24.dp)
        )

        Card(
            modifier = Modifier.fillMaxWidth()
        ) {
            Column(modifier = Modifier.padding(16.dp)) {
                Text(text = "General", style = MaterialTheme.typography.titleMedium)
                Spacer(modifier = Modifier.height(16.dp))

                OutlinedTextField(
                    value = deviceName,
                    onValueChange = { deviceName = it },
                    label = { Text("Device Name") },
                    modifier = Modifier.fillMaxWidth()
                )

                Spacer(modifier = Modifier.height(16.dp))

                Row(
                    modifier = Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.SpaceBetween
                ) {
                    Column {
                        Text(text = "Clipboard Sync", style = MaterialTheme.typography.bodyLarge)
                        Text(text = "Sync clipboard across devices", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.secondary)
                    }
                    Switch(
                        checked = isSyncEnabled,
                        onCheckedChange = { enabled ->
                            isSyncEnabled = enabled
                            val intent = Intent(context, SyncService::class.java)
                            if (enabled) {
                                ContextCompat.startForegroundService(context, intent)
                            } else {
                                context.stopService(intent)
                            }
                        }
                    )
                }

                Spacer(modifier = Modifier.height(24.dp))

                Text(text = "Clipboard History Limit: ${clipboardLimit.toInt()}", style = MaterialTheme.typography.bodyLarge)
                Slider(
                    value = clipboardLimit,
                    onValueChange = { clipboardLimit = it },
                    valueRange = 10f..200f
                )
            }
        }
    }
}

