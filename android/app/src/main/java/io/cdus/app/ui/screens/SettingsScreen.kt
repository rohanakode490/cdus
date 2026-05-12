package io.cdus.app.ui.screens

import android.content.Intent
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import android.os.Build
import android.content.Context
import androidx.core.content.ContextCompat
import io.cdus.app.service.SyncService

@Composable
fun SettingsScreen() {
    val context = LocalContext.current
    val sharedPref = remember { context.getSharedPreferences("cdus_settings", Context.MODE_PRIVATE) }
    
    var isSyncEnabled by remember { 
        mutableStateOf(sharedPref.getBoolean("clipboard_sync", false)) 
    }
    var deviceName by remember { mutableStateOf(Build.MODEL) }
    var clipboardLimit by remember { 
        mutableFloatStateOf(sharedPref.getInt("history_limit", 50).toFloat()) 
    }

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
                    modifier = Modifier.fillMaxWidth(),
                    enabled = false // For now, we use Build.MODEL
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
                            sharedPref.edit().putBoolean("clipboard_sync", enabled).apply()
                            
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
                    onValueChange = { 
                        clipboardLimit = it
                        sharedPref.edit().putInt("history_limit", it.toInt()).apply()
                    },
                    valueRange = 10f..200f
                )
            }
        }
    }
}

