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
import uniffi.cdus_ffi.greetFromRust
import uniffi.cdus_ffi.initLogging
import uniffi.cdus_ffi.initCore
import uniffi.cdus_ffi.registerDevice
import android.net.wifi.WifiManager
import android.content.Context

class MainActivity : ComponentActivity() {
    private var multicastLock: WifiManager.MulticastLock? = null

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
        val identity = initCore(dataDir)
        if (!identity.startsWith("error:")) {
            val parts = identity.split(":", limit = 2)
            if (parts.size >= 2) {
                val nodeId = parts[0]
                val label = parts[1]
                registerDevice(nodeId, label, 5200.toUShort())
                Log.i("CDUS", "Device registered: $nodeId ($label)")
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