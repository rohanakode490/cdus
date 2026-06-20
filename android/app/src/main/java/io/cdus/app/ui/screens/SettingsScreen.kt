package io.cdus.app.ui.screens

import android.content.Intent
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.clickable
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CheckCircle
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
import android.content.SharedPreferences
import io.cdus.app.utils.Logger
import android.net.Uri
import android.provider.Settings
import android.widget.Toast

@Composable
fun SettingsScreen() {
    val context = LocalContext.current
    val sharedPref = remember { context.getSharedPreferences("cdus_settings", Context.MODE_PRIVATE) }
    
    var isSyncEnabled by remember { 
        mutableStateOf(sharedPref.getBoolean("clipboard_sync", false)) 
    }

    DisposableEffect(context) {
        val listener = SharedPreferences.OnSharedPreferenceChangeListener { _, key ->
            if (key == "clipboard_sync") {
                isSyncEnabled = sharedPref.getBoolean("clipboard_sync", false)
            }
        }
        sharedPref.registerOnSharedPreferenceChangeListener(listener)
        onDispose {
            sharedPref.unregisterOnSharedPreferenceChangeListener(listener)
        }
    }
    var deviceName by remember { mutableStateOf(Build.MODEL) }
    var clipboardLimit by remember { 
        mutableFloatStateOf(sharedPref.getInt("history_limit", 50).toFloat()) 
    }
    var developerModeEnabled by remember { 
        mutableStateOf(sharedPref.getBoolean("developer_mode", false)) 
    }
    var tapCount by remember { mutableIntStateOf(0) }

    val powerManager = remember { context.getSystemService(Context.POWER_SERVICE) as android.os.PowerManager }
    var isIgnoringBatteryOptimizations by remember {
        mutableStateOf(
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
                powerManager.isIgnoringBatteryOptimizations(context.packageName)
            } else {
                true
            }
        )
    }

    var isNotificationSyncEnabled by remember {
        mutableStateOf(sharedPref.getBoolean("notification_sync_enabled", false))
    }
    var hasNotificationPermission by remember {
        mutableStateOf(isNotificationServiceEnabled(context))
    }
    var showAppFilterDialog by remember { mutableStateOf(false) }
    val appsList = remember {
        val pm = context.packageManager
        val mainIntent = Intent(Intent.ACTION_MAIN, null).apply { addCategory(Intent.CATEGORY_LAUNCHER) }
        pm.queryIntentActivities(mainIntent, 0).map { resolveInfo ->
            val packageName = resolveInfo.activityInfo.packageName
            val appName = resolveInfo.loadLabel(pm).toString()
            appName to packageName
        }.distinctBy { it.second }.sortedBy { it.first }
    }

    val lifecycleOwner = androidx.lifecycle.compose.LocalLifecycleOwner.current
    DisposableEffect(lifecycleOwner) {
        val observer = androidx.lifecycle.LifecycleEventObserver { _, event ->
            if (event == androidx.lifecycle.Lifecycle.Event.ON_RESUME) {
                isIgnoringBatteryOptimizations = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
                    powerManager.isIgnoringBatteryOptimizations(context.packageName)
                } else {
                    true
                }
                hasNotificationPermission = isNotificationServiceEnabled(context)
            }
        }
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose {
            lifecycleOwner.lifecycle.removeObserver(observer)
        }
    }

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

        Spacer(modifier = Modifier.height(16.dp))

        Card(
            modifier = Modifier.fillMaxWidth()
        ) {
            Column(modifier = Modifier.padding(16.dp)) {
                Text(text = "Background Performance", style = MaterialTheme.typography.titleMedium)
                Spacer(modifier = Modifier.height(8.dp))
                
                Text(
                    text = "Android limits background network and synchronization tasks (Doze mode) to save power. Exempting CDUS from battery optimization ensures instant clipboard sync and file transfer when the screen is off.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
                
                Spacer(modifier = Modifier.height(16.dp))

                Row(
                    modifier = Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.SpaceBetween
                ) {
                    Column(modifier = Modifier.weight(1f).padding(end = 16.dp)) {
                        Text(text = "Run in Background", style = MaterialTheme.typography.bodyLarge)
                        Text(
                            text = if (isIgnoringBatteryOptimizations) "Exempted from battery restrictions" else "Subject to battery optimization",
                            style = MaterialTheme.typography.bodySmall,
                            color = if (isIgnoringBatteryOptimizations) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.secondary
                        )
                    }
                    Switch(
                        checked = isIgnoringBatteryOptimizations,
                        onCheckedChange = { enabled ->
                            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
                                try {
                                    if (enabled) {
                                        val intent = Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS).apply {
                                            data = Uri.parse("package:${context.packageName}")
                                        }
                                        context.startActivity(intent)
                                    } else {
                                        val intent = Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS)
                                        context.startActivity(intent)
                                        Toast.makeText(
                                            context,
                                            "Please find CDUS and select 'Optimize' to enable battery restrictions.",
                                            Toast.LENGTH_LONG
                                        ).show()
                                    }
                                } catch (e: Exception) {
                                    Logger.e("Failed to request battery status change: ${e.message}")
                                }
                            }
                        }
                    )
                }
            }
        }

        Spacer(modifier = Modifier.height(16.dp))

        Card(
            modifier = Modifier.fillMaxWidth()
        ) {
            Column(modifier = Modifier.padding(16.dp)) {
                Text(text = "Notification Sync", style = MaterialTheme.typography.titleMedium)
                Spacer(modifier = Modifier.height(8.dp))
                
                Text(
                    text = "Mirror notifications from your phone to your desktop and synchronize their dismissal status bidirectionally.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
                
                Spacer(modifier = Modifier.height(16.dp))

                Row(
                    modifier = Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.SpaceBetween
                ) {
                    Column(modifier = Modifier.weight(1f).padding(end = 16.dp)) {
                        Text(text = "Sync Notifications", style = MaterialTheme.typography.bodyLarge)
                        Text(
                            text = if (!hasNotificationPermission) "Notification permission required" else if (isNotificationSyncEnabled) "Enabled" else "Disabled",
                            style = MaterialTheme.typography.bodySmall,
                            color = if (hasNotificationPermission && isNotificationSyncEnabled) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.secondary
                        )
                    }
                    Switch(
                        checked = isNotificationSyncEnabled,
                        onCheckedChange = { enabled ->
                            if (enabled && !hasNotificationPermission) {
                                try {
                                    val intent = Intent("android.settings.ACTION_NOTIFICATION_LISTENER_SETTINGS")
                                    context.startActivity(intent)
                                } catch (e: Exception) {
                                    Logger.e("Failed to open notification listener settings: ${e.message}")
                                }
                            } else {
                                isNotificationSyncEnabled = enabled
                                sharedPref.edit().putBoolean("notification_sync_enabled", enabled).apply()
                            }
                        }
                    )
                }

                if (hasNotificationPermission && isNotificationSyncEnabled) {
                    Spacer(modifier = Modifier.height(16.dp))
                    Button(
                        onClick = { showAppFilterDialog = true },
                        modifier = Modifier.fillMaxWidth()
                    ) {
                        Text("Configure Synced Apps")
                    }
                }
            }
        }

        if (showAppFilterDialog) {
            AlertDialog(
                onDismissRequest = { showAppFilterDialog = false },
                title = { Text("Select Synced Apps") },
                text = {
                    Box(modifier = Modifier.heightIn(max = 400.dp)) {
                        androidx.compose.foundation.lazy.LazyColumn {
                            items(appsList.size) { index ->
                                val (appName, pkgName) = appsList[index]
                                var isAppEnabled by remember(pkgName) {
                                    mutableStateOf(sharedPref.getBoolean("notify_app_$pkgName", true))
                                }
                                Row(
                                    modifier = Modifier
                                        .fillMaxWidth()
                                        .clickable {
                                            val newValue = !isAppEnabled
                                            isAppEnabled = newValue
                                            sharedPref.edit().putBoolean("notify_app_$pkgName", newValue).apply()
                                        }
                                        .padding(vertical = 8.dp),
                                    verticalAlignment = Alignment.CenterVertically
                                ) {
                                    Checkbox(
                                        checked = isAppEnabled,
                                        onCheckedChange = { newValue ->
                                            isAppEnabled = newValue
                                            sharedPref.edit().putBoolean("notify_app_$pkgName", newValue).apply()
                                        }
                                    )
                                    Spacer(modifier = Modifier.width(8.dp))
                                    Text(text = appName, style = MaterialTheme.typography.bodyLarge)
                                }
                            }
                        }
                    }
                },
                confirmButton = {
                    TextButton(onClick = { showAppFilterDialog = false }) {
                        Text("Close")
                    }
                }
            )
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

private fun isNotificationServiceEnabled(context: Context): Boolean {
    val pkgName = context.packageName
    val flat = Settings.Secure.getString(context.contentResolver, "enabled_notification_listeners")
    if (!flat.isNullOrEmpty()) {
        val names = flat.split(":")
        for (name in names) {
            val cn = android.content.ComponentName.unflattenFromString(name)
            if (cn != null && cn.packageName == pkgName) {
                return true
            }
        }
    }
    return false
}

