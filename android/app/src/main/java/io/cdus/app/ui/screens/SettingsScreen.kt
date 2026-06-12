package io.cdus.app.ui.screens

import android.content.Intent
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.clickable
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Delete
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
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import uniffi.cdus_ffi.SyncedFolderRecord
import uniffi.cdus_ffi.ConflictedFileRecord
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

@Composable
fun SettingsScreen() {
    val context = LocalContext.current
    val sharedPref = remember { context.getSharedPreferences("cdus_settings", Context.MODE_PRIVATE) }
    
    var syncedFolders by remember { mutableStateOf<List<SyncedFolderRecord>>(emptyList()) }
    var selectedFolderUri by remember { mutableStateOf<String?>(null) }
    var showLabelDialog by remember { mutableStateOf(false) }
    var showConflictDialog by remember { mutableStateOf(false) }
    var activeConflictFolderId by remember { mutableStateOf<Long?>(null) }
    val scope = rememberCoroutineScope()

    fun refreshFolders() {
        scope.launch(Dispatchers.IO) {
            try {
                syncedFolders = uniffi.cdus_ffi.getSyncedFolders()
            } catch (e: Exception) {
                Logger.e("Failed to get synced folders: ${e.message}")
            }
        }
    }

    LaunchedEffect(Unit) {
        refreshFolders()
    }

    val folderPickerLauncher = rememberLauncherForActivityResult(
        contract = ActivityResultContracts.OpenDocumentTree()
    ) { uri: Uri? ->
        if (uri != null) {
            try {
                context.contentResolver.takePersistableUriPermission(
                    uri,
                    Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION
                )
                selectedFolderUri = uri.toString()
                showLabelDialog = true
            } catch (e: Exception) {
                Logger.e("Failed to take persistable URI permission: ${e.message}")
                Toast.makeText(context, "Failed to get persistent folder access", Toast.LENGTH_SHORT).show()
            }
        }
    }

    // Label Dialog Layout
    if (showLabelDialog) {
        var labelText by remember { mutableStateOf("") }
        AlertDialog(
            onDismissRequest = { 
                showLabelDialog = false
                selectedFolderUri = null
            },
            title = { Text("Name Synced Folder") },
            text = {
                Column {
                    Text("Enter a friendly label for this folder:")
                    Spacer(modifier = Modifier.height(8.dp))
                    OutlinedTextField(
                        value = labelText,
                        onValueChange = { labelText = it },
                        placeholder = { Text("e.g. Documents, Backup") },
                        modifier = Modifier.fillMaxWidth()
                    )
                }
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        val path = selectedFolderUri ?: ""
                        val label = labelText.trim().ifEmpty { "Synced Folder" }
                        scope.launch(Dispatchers.IO) {
                            try {
                                uniffi.cdus_ffi.addSyncedFolder(path, label)
                                refreshFolders()
                            } catch (e: Exception) {
                                Logger.e("Failed to add folder: ${e.message}")
                            }
                        }
                        showLabelDialog = false
                        selectedFolderUri = null
                    }
                ) {
                    Text("Add")
                }
            },
            dismissButton = {
                TextButton(
                    onClick = {
                        showLabelDialog = false
                        selectedFolderUri = null
                    }
                ) {
                    Text("Cancel")
                }
            }
        )
    }

    // Conflict Resolution Dialog Layout
    if (showConflictDialog && activeConflictFolderId != null) {
        var conflicts by remember { mutableStateOf<List<ConflictedFileRecord>>(emptyList()) }
        
        LaunchedEffect(activeConflictFolderId) {
            scope.launch(Dispatchers.IO) {
                try {
                    conflicts = uniffi.cdus_ffi.getConflictedFiles(activeConflictFolderId!!)
                } catch (e: Exception) {
                    Logger.e("Failed to fetch conflicts: ${e.message}")
                }
            }
        }

        AlertDialog(
            onDismissRequest = {
                showConflictDialog = false
                activeConflictFolderId = null
                refreshFolders()
            },
            title = { Text("Conflict Resolution") },
            text = {
                Column(
                    modifier = Modifier
                        .fillMaxWidth()
                        .heightIn(max = 400.dp)
                        .verticalScroll(rememberScrollState())
                ) {
                    if (conflicts.isEmpty()) {
                        Text("No active conflicts found.", style = MaterialTheme.typography.bodyMedium)
                    } else {
                        conflicts.forEach { conflict ->
                            Card(
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .padding(vertical = 8.dp),
                                colors = CardDefaults.cardColors(
                                    containerColor = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.5f)
                                )
                            ) {
                                Column(modifier = Modifier.padding(12.dp)) {
                                    Text(
                                        text = conflict.filePath,
                                        style = MaterialTheme.typography.titleSmall,
                                        color = MaterialTheme.colorScheme.primary
                                    )
                                    Spacer(modifier = Modifier.height(8.dp))
                                    
                                    Row(modifier = Modifier.fillMaxWidth()) {
                                        Column(modifier = Modifier.weight(1f)) {
                                            Text("Local Version", style = MaterialTheme.typography.labelMedium)
                                            Text("Size: ${formatBytes(conflict.localSize.toLong())}", style = MaterialTheme.typography.bodySmall)
                                            Text("Mod: ${conflict.localModified}", style = MaterialTheme.typography.bodySmall)
                                        }
                                        Spacer(modifier = Modifier.width(8.dp))
                                        Column(modifier = Modifier.weight(1f)) {
                                            Text("Remote Version", style = MaterialTheme.typography.labelMedium)
                                            Text("Device: ${conflict.remoteDeviceName}", style = MaterialTheme.typography.bodySmall)
                                            Text("Size: ${formatBytes(conflict.remoteSize.toLong())}", style = MaterialTheme.typography.bodySmall)
                                            Text("Mod: ${conflict.remoteModified}", style = MaterialTheme.typography.bodySmall)
                                        }
                                    }
                                    Spacer(modifier = Modifier.height(12.dp))
                                    Row(
                                        modifier = Modifier.fillMaxWidth(),
                                        horizontalArrangement = Arrangement.SpaceBetween
                                    ) {
                                        TextButton(
                                            onClick = {
                                                scope.launch(Dispatchers.IO) {
                                                    try {
                                                        uniffi.cdus_ffi.resolveConflict(conflict.id)
                                                        conflicts = uniffi.cdus_ffi.getConflictedFiles(activeConflictFolderId!!)
                                                        if (conflicts.isEmpty()) {
                                                            showConflictDialog = false
                                                            activeConflictFolderId = null
                                                            refreshFolders()
                                                        }
                                                    } catch (e: Exception) {
                                                        Logger.e("Failed to resolve conflict: ${e.message}")
                                                    }
                                                }
                                            }
                                        ) {
                                            Text("Keep Local", style = MaterialTheme.typography.labelSmall)
                                        }
                                        TextButton(
                                            onClick = {
                                                scope.launch(Dispatchers.IO) {
                                                    try {
                                                        uniffi.cdus_ffi.resolveConflict(conflict.id)
                                                        conflicts = uniffi.cdus_ffi.getConflictedFiles(activeConflictFolderId!!)
                                                        if (conflicts.isEmpty()) {
                                                            showConflictDialog = false
                                                            activeConflictFolderId = null
                                                            refreshFolders()
                                                        }
                                                    } catch (e: Exception) {
                                                        Logger.e("Failed to resolve conflict: ${e.message}")
                                                    }
                                                }
                                            }
                                        ) {
                                            Text("Keep Remote", style = MaterialTheme.typography.labelSmall)
                                        }
                                    }
                                    Button(
                                        onClick = {
                                            scope.launch(Dispatchers.IO) {
                                                try {
                                                    uniffi.cdus_ffi.resolveConflict(conflict.id)
                                                    conflicts = uniffi.cdus_ffi.getConflictedFiles(activeConflictFolderId!!)
                                                    if (conflicts.isEmpty()) {
                                                        showConflictDialog = false
                                                        activeConflictFolderId = null
                                                        refreshFolders()
                                                    }
                                                } catch (e: Exception) {
                                                    Logger.e("Failed to resolve conflict: ${e.message}")
                                                }
                                            }
                                        },
                                        modifier = Modifier.fillMaxWidth().padding(top = 4.dp),
                                        colors = ButtonDefaults.buttonColors(containerColor = MaterialTheme.colorScheme.secondary)
                                    ) {
                                        Text("Keep Both", style = MaterialTheme.typography.labelSmall)
                                    }
                                }
                            }
                        }
                    }
                }
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        showConflictDialog = false
                        activeConflictFolderId = null
                        refreshFolders()
                    }
                ) {
                    Text("Close")
                }
            }
        )
    }
    
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

    val lifecycleOwner = androidx.lifecycle.compose.LocalLifecycleOwner.current
    DisposableEffect(lifecycleOwner) {
        val observer = androidx.lifecycle.LifecycleEventObserver { _, event ->
            if (event == androidx.lifecycle.Lifecycle.Event.ON_RESUME) {
                isIgnoringBatteryOptimizations = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
                    powerManager.isIgnoringBatteryOptimizations(context.packageName)
                } else {
                    true
                }
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
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.SpaceBetween
                ) {
                    Text(text = "Folder Synchronization", style = MaterialTheme.typography.titleMedium)
                    Button(
                        onClick = { folderPickerLauncher.launch(null) }
                    ) {
                        Text("Add Folder")
                    }
                }
                Spacer(modifier = Modifier.height(8.dp))
                Text(
                    text = "Select specific folders to synchronize with your other devices.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
                Spacer(modifier = Modifier.height(16.dp))

                if (syncedFolders.isEmpty()) {
                    Text(
                        text = "No folders synced yet.",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.outline,
                        modifier = Modifier.align(Alignment.CenterHorizontally).padding(vertical = 16.dp)
                    )
                } else {
                    syncedFolders.forEach { folder ->
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(vertical = 8.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.SpaceBetween
                        ) {
                            Column(modifier = Modifier.weight(1f).padding(end = 8.dp)) {
                                Text(text = folder.label, style = MaterialTheme.typography.bodyLarge)
                                Text(
                                    text = folder.path,
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.outline,
                                    maxLines = 1,
                                    overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis
                                )
                            }
                            
                            val statusColor = when (folder.status.lowercase()) {
                                "synced" -> MaterialTheme.colorScheme.primary
                                "syncing" -> MaterialTheme.colorScheme.secondary
                                "conflict" -> MaterialTheme.colorScheme.error
                                else -> MaterialTheme.colorScheme.outline
                            }

                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Text(
                                    text = folder.status.uppercase(),
                                    style = MaterialTheme.typography.labelSmall,
                                    color = statusColor,
                                    modifier = Modifier.padding(end = 8.dp)
                                )
                                if (folder.status.lowercase() == "conflict") {
                                    TextButton(
                                        onClick = {
                                            activeConflictFolderId = folder.id
                                            showConflictDialog = true
                                        },
                                        modifier = Modifier.padding(end = 4.dp)
                                    ) {
                                        Text("Resolve", style = MaterialTheme.typography.labelSmall)
                                    }
                                }
                                IconButton(
                                    onClick = {
                                        scope.launch(Dispatchers.IO) {
                                            try {
                                                uniffi.cdus_ffi.removeSyncedFolder(folder.id)
                                                refreshFolders()
                                            } catch (e: Exception) {
                                                Logger.e("Failed to remove folder: ${e.message}")
                                            }
                                        }
                                    }
                                ) {
                                    Icon(
                                        imageVector = Icons.Default.Delete,
                                        contentDescription = "Remove Folder",
                                        tint = MaterialTheme.colorScheme.error
                                    )
                                }
                            }
                        }
                    }
                }
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

fun formatBytes(bytes: Long): String {
    if (bytes <= 0) return "0 B"
    val units = arrayOf("B", "KB", "MB", "GB", "TB")
    val digitGroups = (Math.log10(bytes.toDouble()) / Math.log10(1024.0)).toInt()
    if (digitGroups < 0 || digitGroups >= units.size) return "$bytes B"
    return String.format(java.util.Locale.US, "%.2f %s", bytes / Math.pow(1024.0, digitGroups.toDouble()), units[digitGroups])
}

