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

import android.util.Log
import android.content.Intent
import android.os.Build
import android.content.ClipboardManager
import uniffi.cdus_ffi.greetFromRust
import uniffi.cdus_ffi.initLogging
import uniffi.cdus_ffi.initCore
import uniffi.cdus_ffi.registerDevice
import android.net.wifi.WifiManager
import android.content.Context

class MainActivity : ComponentActivity() {
    private var multicastLock: WifiManager.MulticastLock? = null

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (hasFocus) {
            Log.d("CDUS", "Window gained focus, checking clipboard")
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
                                Log.i("CDUS", "New clipboard content detected on resume, broadcasting")
                                uniffi.cdus_ffi.broadcastClipboard(content)
                            }
                        }
                    }
                }
            }
        } catch (e: Exception) {
            Log.e("CDUS", "Error checking clipboard on resume: ${e.message}")
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
                Log.i("CDUS", "Device registered: $nodeId ($label)")
                
                // Start sync service if enabled
                val sharedPref = getSharedPreferences("cdus_settings", Context.MODE_PRIVATE)
                if (sharedPref.getBoolean("clipboard_sync", false)) {
                    val intent = Intent(this, io.cdus.app.service.SyncService::class.java)
                    androidx.core.content.ContextCompat.startForegroundService(this, intent)
                }
            }
        } else {
            Log.e("CDUS", "Failed to init core: $identity")
        }
        
        val greeting = greetFromRust("Android")
        Log.d("CDUS", "Rust says: $greeting")
        
        enableEdgeToEdge()
        setContent {
            CdusandroidTheme {
                MainScreen()
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
fun MainScreen() {
    val navController = rememberNavController()
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