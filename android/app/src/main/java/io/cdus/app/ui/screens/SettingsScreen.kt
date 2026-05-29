package io.cdus.app.ui.screens

import android.content.Intent
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.clickable
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
    var developerModeEnabled by remember { 
        mutableStateOf(sharedPref.getBoolean("developer_mode", false)) 
    }
    var tapCount by remember { mutableIntStateOf(0) }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp)
            .verticalScroll(rememberScrollState())
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

        if (developerModeEnabled) {
            Spacer(modifier = Modifier.height(24.dp))
            Text(text = "Developer Options", style = MaterialTheme.typography.titleMedium, color = MaterialTheme.colorScheme.primary)
            Spacer(modifier = Modifier.height(8.dp))
            Card(
                modifier = Modifier.fillMaxWidth(),
                colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.primaryContainer.copy(alpha = 0.3f))
            ) {
                Column(modifier = Modifier.padding(16.dp)) {
                    Text(text = "Network Benchmark", style = MaterialTheme.typography.labelLarge)
                    Text(
                        text = "The 1GB synthetic benchmark can be triggered from the Devices screen for any online peer.",
                        style = MaterialTheme.typography.bodySmall
                    )
                    Spacer(modifier = Modifier.height(8.dp))
                    Button(
                        onClick = { 
                            developerModeEnabled = false
                            sharedPref.edit().putBoolean("developer_mode", false).apply()
                        },
                        colors = ButtonDefaults.buttonColors(containerColor = MaterialTheme.colorScheme.error)
                    ) {
                        Text("Disable Developer Mode")
                    }
                }
            }
        }

        Spacer(modifier = Modifier.weight(1f))
        
        Box(
            modifier = Modifier.fillMaxWidth().padding(vertical = 16.dp),
            contentAlignment = Alignment.Center
        ) {
            Text(
                text = "Version 0.1.0-alpha",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.outline,
                modifier = Modifier.clickable {
                    tapCount++
                    if (tapCount >= 7 && !developerModeEnabled) {
                        developerModeEnabled = true
                        sharedPref.edit().putBoolean("developer_mode", true).apply()
                        tapCount = 0
                        // Could add a Toast here
                    }
                }
            )
        }
    }
}

