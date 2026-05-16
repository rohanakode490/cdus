package io.cdus.app

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Icon
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.navigation.NavDestination.Companion.hierarchy
import androidx.navigation.NavGraph.Companion.findStartDestination
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.currentBackStackEntryAsState
import androidx.navigation.compose.rememberNavController
import io.cdus.app.ui.navigation.Screen
import io.cdus.app.ui.navigation.navItems
import io.cdus.app.ui.screens.ClipboardScreen
import io.cdus.app.ui.screens.DevicesScreen
import io.cdus.app.ui.screens.FilesScreen
import io.cdus.app.ui.screens.SettingsScreen
import io.cdus.app.ui.theme.CdusandroidTheme

import android.content.Intent
import android.os.Build
import android.content.ClipboardManager
import uniffi.cdus_ffi.greetFromRust
import uniffi.cdus_ffi.initLogging
import uniffi.cdus_ffi.initCore
import uniffi.cdus_ffi.registerDevice
import android.net.wifi.WifiManager
import android.content.Context

import io.cdus.app.utils.FileUtils
import io.cdus.app.utils.Logger
import io.cdus.app.ui.components.DevicePickerDialog
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue

class MainActivity : ComponentActivity() {
    private var multicastLock: WifiManager.MulticastLock? = null
    private var sharedFilePath by mutableStateOf<String?>(null)

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (hasFocus) {
            Logger.d("Window gained focus, checking clipboard")
            checkClipboard()
        }
    }

    override fun onResume() {
        super.onResume()
        // Still check here for robustness
        checkClipboard()
    }

    private fun checkClipboard() {
        try {
            val clipboard = getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
            if (clipboard.hasPrimaryClip()) {
                val clipData = clipboard.primaryClip
                if (clipData != null && clipData.itemCount > 0) {
                    val content = clipData.getItemAt(0).text?.toString()
                    if (content != null) {
                        val sharedPref = getSharedPreferences("cdus_settings", Context.MODE_PRIVATE)
                        if (sharedPref.getBoolean("clipboard_sync", false)) {
                            // Only broadcast if it's new
                            val lastHash = sharedPref.getString("last_clip_hash", "")
                            val currentHash = content.hashCode().toString()
                            if (currentHash != lastHash) {
                                sharedPref.edit().putString("last_clip_hash", currentHash).apply()
                                Logger.i("New clipboard content detected on resume, broadcasting")
                                uniffi.cdus_ffi.broadcastClipboard(content)
                            }
                        }
                    }
                }
            }
        } catch (e: Exception) {
            Logger.e("Error checking clipboard on resume: ${e.message}")
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handleIntent(intent)
    }

    private fun handleIntent(intent: Intent?) {
        if (intent?.action == Intent.ACTION_SEND) {
            if ("text/plain" == intent.type) {
                intent.getStringExtra(Intent.EXTRA_TEXT)?.let { text ->
                    Logger.i("Shared text received: $text")
                    // Pre-fill or broadcast directly
                    uniffi.cdus_ffi.broadcastClipboard(text)
                }
            } else {
                val uri = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    intent.getParcelableExtra(Intent.EXTRA_STREAM, android.net.Uri::class.java)
                } else {
                    @Suppress("DEPRECATION")
                    intent.getParcelableExtra(Intent.EXTRA_STREAM)
                }

                uri?.let { fileUri ->
                    Logger.i("Shared file URI received: $fileUri")
                    val path = FileUtils.copyUriToLocal(this, fileUri)
                    if (path != null) {
                        Logger.i("File copied to: $path")
                        sharedFilePath = path
                    }
                }
            }
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        
        initLogging()
        
        val wifi = getSystemService(Context.WIFI_SERVICE) as WifiManager
        multicastLock = wifi.createMulticastLock("cdus_multicast_lock").apply {
            setReferenceCounted(true)
            acquire()
        }

        // Initialize core and register for mDNS
        val dataDir = filesDir.absolutePath
        val deviceName = Build.MODEL
        val identity = initCore(dataDir, deviceName)
        if (!identity.startsWith("error:")) {
            val parts = identity.split(":", limit = 2)
            if (parts.size >= 2) {
                val nodeId = parts[0]
                val label = parts[1]
                registerDevice(nodeId, label, 5200.toUShort())
                Logger.i("Device registered: $nodeId ($label)")
                
                // Start sync service if enabled (always start for now to handle file transfers)
                val intent = Intent(this, io.cdus.app.service.SyncService::class.java)
                androidx.core.content.ContextCompat.startForegroundService(this, intent)
                Logger.i("SyncService start requested")
            }
        } else {
            Logger.e("Failed to init core: $identity")
        }

        handleIntent(intent)
        
        val greeting = greetFromRust("Android")
        Logger.d("Rust says: $greeting")
        
        enableEdgeToEdge()
        setContent {
            CdusandroidTheme {
                MainScreen(
                    sharedFilePath = sharedFilePath,
                    onFileSent = { sharedFilePath = null }
                )
            }
        }
    }

    override fun onDestroy() {
        super.onDestroy()
        multicastLock?.let {
            if (it.isHeld) {
                it.release()
            }
        }
    }
}

@Composable
fun MainScreen(sharedFilePath: String?, onFileSent: () -> Unit) {
    val navController = rememberNavController()
    
    if (sharedFilePath != null) {
        DevicePickerDialog(
            onDeviceSelected = { nodeId ->
                uniffi.cdus_ffi.sendFile(nodeId, sharedFilePath)
                onFileSent()
            },
            onDismiss = onFileSent
        )
    }

    Scaffold(
        bottomBar = {
            NavigationBar {
                val navBackStackEntry by navController.currentBackStackEntryAsState()
                val currentDestination = navBackStackEntry?.destination
                navItems.forEach { screen ->
                    NavigationBarItem(
                        icon = { Icon(screen.icon, contentDescription = screen.title) },
                        label = { Text(screen.title) },
                        selected = currentDestination?.hierarchy?.any { it.route == screen.route } == true,
                        onClick = {
                            navController.navigate(screen.route) {
                                popUpTo(navController.graph.findStartDestination().id) {
                                    saveState = true
                                }
                                launchSingleTop = true
                                restoreState = true
                            }
                        }
                    )
                }
            }
        }
    ) { innerPadding ->
        NavHost(
            navController = navController,
            startDestination = Screen.Devices.route,
            modifier = Modifier.padding(innerPadding)
        ) {
            composable(Screen.Devices.route) { DevicesScreen() }
            composable(Screen.Clipboard.route) { ClipboardScreen() }
            composable(Screen.Files.route) { FilesScreen() }
            composable(Screen.Settings.route) { SettingsScreen() }
        }
    }
}