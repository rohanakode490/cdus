package io.cdus.app.ui.screens

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.border
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.foundation.clickable
import androidx.compose.foundation.background
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Computer
import androidx.compose.material.icons.filled.Smartphone
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material.icons.filled.CloudOff
import androidx.compose.material.icons.filled.QrCode
import androidx.compose.material.icons.filled.QrCodeScanner
import androidx.compose.material.icons.filled.Devices
import androidx.compose.material.icons.filled.Send
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import androidx.compose.animation.core.*
import androidx.compose.ui.graphics.graphicsLayer
import uniffi.cdus_ffi.startDiscovery
import uniffi.cdus_ffi.stopDiscovery
import uniffi.cdus_ffi.getDiscoveredDevices
import uniffi.cdus_ffi.DiscoveredDevice
import uniffi.cdus_ffi.PairingStatus
import uniffi.cdus_ffi.getPairingStatus
import uniffi.cdus_ffi.initiatePairing
import uniffi.cdus_ffi.confirmPairing
import uniffi.cdus_ffi.cancelPairing

import uniffi.cdus_ffi.clearDiscoveredDevices
import uniffi.cdus_ffi.getPairedDevices
import uniffi.cdus_ffi.unpairDevice
import uniffi.cdus_ffi.PairedDevice
import uniffi.cdus_ffi.sendFile
import uniffi.cdus_ffi.startBenchmark
import uniffi.cdus_ffi.getQrPairingPayload
import uniffi.cdus_ffi.pairWithQr
import io.cdus.app.utils.FileUtils
import io.cdus.app.utils.Logger
import io.cdus.app.utils.UIUtils
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.graphics.asImageBitmap
import com.google.zxing.BarcodeFormat
import com.google.zxing.qrcode.QRCodeWriter
import android.graphics.Bitmap
import android.graphics.Color as AndroidColor
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import com.google.android.gms.tasks.Tasks
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.barcode.BarcodeScanning
import com.google.mlkit.vision.common.InputImage
import java.util.concurrent.TimeUnit
import java.util.concurrent.Executors

import androidx.compose.foundation.Image
import androidx.compose.ui.draw.clip
import androidx.lifecycle.compose.LocalLifecycleOwner

enum class ConnectionStatus {
    ONLINE, OFFLINE, RECONNECTING, CONNECTING
}

data class DeviceConnectionState(
    val status: ConnectionStatus,
    val transport: String?, // "LAN", "Relay", null
    val countdown: Int = 0
)

@Composable
fun DevicesScreen() {
    var isScanning by remember { mutableStateOf(false) }
    var discoveredDevices by remember { mutableStateOf<List<DiscoveredDevice>>(emptyList()) }
    var pairedDevices by remember { mutableStateOf<List<PairedDevice>>(emptyList()) }
    var pairingStatus by remember { mutableStateOf<PairingStatus?>(null) }
    var isDeveloperMode by remember { mutableStateOf(false) }
    var showQrDialog by remember { mutableStateOf(false) }
    var showScannerDialog by remember { mutableStateOf(false) }
    var isLoading by remember { mutableStateOf(true) }
    var errorMsg by remember { mutableStateOf<String?>(null) }

    val context = LocalContext.current
    
    // --- Reconnection & Relay Dialog States ---
    val connectionStates = remember { mutableStateMapOf<String, DeviceConnectionState>() }
    var showRelayErrorDialog by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    fun triggerActualConnect(deviceId: String) {
        connectionStates[deviceId] = DeviceConnectionState(ConnectionStatus.CONNECTING, null, 0)
        scope.launch {
            try {
                uniffi.cdus_ffi.initiatePairing(deviceId)
            } catch (e: Exception) {
                Logger.e("Manual reconnect error: ${e.message}")
                connectionStates[deviceId] = DeviceConnectionState(ConnectionStatus.OFFLINE, null, 0)
                return@launch
            }
            delay(15000)
            val state = connectionStates[deviceId]
            if (state != null && state.status == ConnectionStatus.CONNECTING) {
                connectionStates[deviceId] = DeviceConnectionState(ConnectionStatus.OFFLINE, null, 0)
            }
        }
    }

    val cameraPermissionLauncher = rememberLauncherForActivityResult(
        contract = ActivityResultContracts.RequestPermission()
    ) { isGranted: Boolean ->
        if (isGranted) {
            showScannerDialog = true
        } else {
            android.widget.Toast.makeText(context, "Camera permission required for QR scanning", android.widget.Toast.LENGTH_SHORT).show()
        }
    }
    
    val sharedPref = remember { context.getSharedPreferences("cdus_settings", android.content.Context.MODE_PRIVATE) }
    
    var selectedDeviceForFile by remember { mutableStateOf<String?>(null) }

    val filePickerLauncher = rememberLauncherForActivityResult(
        contract = ActivityResultContracts.GetContent()
    ) { uri: android.net.Uri? ->
        uri?.let {
            val deviceId = selectedDeviceForFile ?: return@let
            val path = FileUtils.copyUriToLocal(context, it)
            if (path != null) {
                sendFile(deviceId, path)
                android.widget.Toast.makeText(context, "Sending file...", android.widget.Toast.LENGTH_SHORT).show()
            }
        }
        selectedDeviceForFile = null
    }

    DisposableEffect(isScanning) {
        if (isScanning) {
            io.cdus.app.CoreInitializer.acquireMulticastLock(context)
        }
        onDispose {
            io.cdus.app.CoreInitializer.releaseMulticastLock()
        }
    }

    LaunchedEffect(isScanning) {
        if (isScanning) {
            clearDiscoveredDevices()
            discoveredDevices = emptyList()
            startDiscovery()
            while (isActive) {
                discoveredDevices = getDiscoveredDevices()
                delay(1000)
            }
        } else {
            stopDiscovery()
        }
    }

    LaunchedEffect(Unit) {
        while (isActive) {
            try {
                pairingStatus = getPairingStatus()
                val devices = getPairedDevices()
                pairedDevices = devices
                isDeveloperMode = sharedPref.getBoolean("developer_mode", false)
                io.cdus.app.data.DeviceManager.updateLabels(devices)
                errorMsg = null
                
                // Sync connection states for devices with real connectivity status
                devices.forEach { device ->
                    val state = connectionStates[device.nodeId]
                    if (device.isOnline) {
                        if (state == null || state.status != ConnectionStatus.ONLINE) {
                            connectionStates[device.nodeId] = DeviceConnectionState(ConnectionStatus.ONLINE, "LAN", 0)
                        }
                    } else {
                        if (state == null || state.status == ConnectionStatus.ONLINE) {
                            connectionStates[device.nodeId] = DeviceConnectionState(ConnectionStatus.OFFLINE, null, 0)
                        }
                    }
                }
            } catch (e: Exception) {
                errorMsg = e.message ?: "Failed to load paired devices"
            } finally {
                isLoading = false
            }
            delay(1000)
        }
    }

    if (pairingStatus != null && pairingStatus!!.active && !pairingStatus!!.silent) {
        PairingDialog(
            status = pairingStatus!!,
            onDismiss = { cancelPairing() },
            onConfirm = { confirmPairing(true) },
            onDecline = { confirmPairing(false) }
        )
    }

    if (showQrDialog) {
        QrPairingDialog(
            onDismiss = { showQrDialog = false }
        )
    }

    if (showScannerDialog) {
        QrScannerDialog(
            onDismiss = { showScannerDialog = false },
            onQrScanned = { payload ->
                Logger.i("Handling scanned QR payload...")
                showScannerDialog = false
                try {
                    pairWithQr(payload)
                    android.widget.Toast.makeText(context, "Pairing with QR...", android.widget.Toast.LENGTH_SHORT).show()
                    Logger.i("pairWithQr called successfully")
                } catch (e: Exception) {
                    Logger.e("Error calling pairWithQr: ${e.message}")
                    android.widget.Toast.makeText(context, "Pairing failed: ${e.message}", android.widget.Toast.LENGTH_LONG).show()
                }
            }
        )
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp)
    ) {
        val isRelayConnected by remember { io.cdus.app.data.DeviceManager.isRelayConnected }
        val relayError by remember { io.cdus.app.data.DeviceManager.relayError }

        if (showRelayErrorDialog) {
            AlertDialog(
                onDismissRequest = { showRelayErrorDialog = false },
                icon = { Icon(Icons.Default.CloudOff, contentDescription = null, tint = MaterialTheme.colorScheme.error) },
                title = { Text("Relay Offline") },
                text = {
                    Text(
                        text = "CDUS lost connection to the remote relay server. Remote sync will be unavailable, but local network (LAN) sync will continue to function.",
                        style = MaterialTheme.typography.bodyMedium
                    )
                },
                confirmButton = {
                    TextButton(onClick = {
                        showRelayErrorDialog = false
                        try {
                            uniffi.cdus_ffi.connectRelay()
                        } catch (e: Exception) {
                            Logger.e("Failed to reconnect relay: ${e.message}")
                        }
                    }) {
                        Text("Retry")
                    }
                },
                dismissButton = {
                    TextButton(onClick = { showRelayErrorDialog = false }) {
                        Text("Close")
                    }
                }
            )
        }

        // Header & Live Mesh Status
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(bottom = 16.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween
        ) {
            Column {
                Text(
                    text = "Devices",
                    style = MaterialTheme.typography.headlineMedium,
                    color = MaterialTheme.colorScheme.onBackground
                )
                Text(
                    text = "Encrypted Cross-Device Mesh",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }

            // Mesh / Relay Status Badge
            Surface(
                shape = RoundedCornerShape(16.dp),
                color = if (isRelayConnected) MaterialTheme.colorScheme.secondaryContainer else MaterialTheme.colorScheme.tertiaryContainer,
                modifier = Modifier
                    .clickable {
                        if (!isRelayConnected) {
                            showRelayErrorDialog = true
                        }
                    }
                    .border(
                        1.dp,
                        if (isRelayConnected) MaterialTheme.colorScheme.secondary.copy(alpha = 0.4f) else MaterialTheme.colorScheme.tertiary.copy(alpha = 0.4f),
                        RoundedCornerShape(16.dp)
                    )
            ) {
                Row(
                    modifier = Modifier.padding(horizontal = 10.dp, vertical = 6.dp),
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Box(
                        modifier = Modifier
                            .size(7.dp)
                            .background(
                                color = if (isRelayConnected) MaterialTheme.colorScheme.secondary else MaterialTheme.colorScheme.tertiary,
                                shape = CircleShape
                            )
                    )
                    Spacer(modifier = Modifier.width(6.dp))
                    Text(
                        text = if (isRelayConnected) "Direct + Relay Active" else "LAN Only (Relay Off)",
                        style = MaterialTheme.typography.labelSmall,
                        color = if (isRelayConnected) MaterialTheme.colorScheme.onSecondaryContainer else MaterialTheme.colorScheme.onTertiaryContainer,
                        fontWeight = FontWeight.SemiBold
                    )
                }
            }
        }

        LazyColumn(
            modifier = Modifier.fillMaxSize(),
            verticalArrangement = Arrangement.spacedBy(16.dp)
        ) {
            // Section 1: Connect / Pairing Actions Hub
            item {
                Card(
                    modifier = Modifier
                        .fillMaxWidth()
                        .border(1.dp, MaterialTheme.colorScheme.outline, RoundedCornerShape(14.dp)),
                    shape = RoundedCornerShape(14.dp),
                    colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceVariant)
                ) {
                    Column(
                        modifier = Modifier.padding(14.dp),
                        verticalArrangement = Arrangement.spacedBy(10.dp)
                    ) {
                        Text(
                            text = "Pair New Device",
                            style = MaterialTheme.typography.titleMedium,
                            color = MaterialTheme.colorScheme.onSurface
                        )
                        Text(
                            text = "Direct peer-to-peer encryption with Noise XX handshake.",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )

                        // Primary Action: Scan QR
                        Button(
                            onClick = { cameraPermissionLauncher.launch(android.Manifest.permission.CAMERA) },
                            modifier = Modifier
                                .fillMaxWidth()
                                .height(46.dp),
                            shape = RoundedCornerShape(10.dp),
                            colors = ButtonDefaults.buttonColors(
                                containerColor = MaterialTheme.colorScheme.primary,
                                contentColor = MaterialTheme.colorScheme.onPrimary
                            )
                        ) {
                            Icon(
                                imageVector = Icons.Default.QrCodeScanner,
                                contentDescription = null,
                                modifier = Modifier.size(18.dp)
                            )
                            Spacer(modifier = Modifier.width(8.dp))
                            Text("Scan Pairing QR", fontWeight = FontWeight.SemiBold)
                        }

                        // Secondary Actions Row
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.spacedBy(8.dp)
                        ) {
                            OutlinedButton(
                                onClick = { showQrDialog = true },
                                modifier = Modifier
                                    .weight(1f)
                                    .height(42.dp),
                                shape = RoundedCornerShape(10.dp)
                            ) {
                                Icon(
                                    imageVector = Icons.Default.QrCode,
                                    contentDescription = null,
                                    modifier = Modifier.size(16.dp)
                                )
                                Spacer(modifier = Modifier.width(6.dp))
                                Text("My QR")
                            }

                            OutlinedButton(
                                onClick = { isScanning = !isScanning },
                                modifier = Modifier
                                    .weight(1.3f)
                                    .height(42.dp),
                                shape = RoundedCornerShape(10.dp)
                            ) {
                                if (isScanning) {
                                    CircularProgressIndicator(
                                        modifier = Modifier.size(16.dp),
                                        strokeWidth = 2.dp,
                                        color = MaterialTheme.colorScheme.primary
                                    )
                                    Spacer(modifier = Modifier.width(6.dp))
                                    Text("Stop Scan")
                                } else {
                                    Icon(
                                        imageVector = Icons.Default.Refresh,
                                        contentDescription = null,
                                        modifier = Modifier.size(16.dp)
                                    )
                                    Spacer(modifier = Modifier.width(6.dp))
                                    Text("Scan LAN")
                                }
                            }
                        }
                    }
                }
            }

            // Section 2: Paired Devices
            item {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.SpaceBetween
                ) {
                    Text(
                        text = "Paired Devices",
                        style = MaterialTheme.typography.titleMedium,
                        color = MaterialTheme.colorScheme.onBackground
                    )
                    if (pairedDevices.isNotEmpty()) {
                        Surface(
                            shape = RoundedCornerShape(8.dp),
                            color = MaterialTheme.colorScheme.surfaceVariant
                        ) {
                            Text(
                                text = "${pairedDevices.size} connected",
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                modifier = Modifier.padding(horizontal = 8.dp, vertical = 2.dp)
                            )
                        }
                    }
                }
            }

            if (isLoading) {
                item {
                    Box(
                        modifier = Modifier
                            .fillMaxWidth()
                            .height(80.dp),
                        contentAlignment = Alignment.Center
                    ) {
                        CircularProgressIndicator(modifier = Modifier.size(24.dp))
                    }
                }
            } else if (errorMsg != null) {
                item {
                    Card(
                        modifier = Modifier
                            .fillMaxWidth()
                            .border(1.dp, MaterialTheme.colorScheme.error.copy(alpha = 0.4f), RoundedCornerShape(12.dp)),
                        shape = RoundedCornerShape(12.dp),
                        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.errorContainer)
                    ) {
                        Column(
                            modifier = Modifier.padding(16.dp),
                            horizontalAlignment = Alignment.CenterHorizontally
                        ) {
                            Icon(Icons.Default.Warning, contentDescription = null, tint = MaterialTheme.colorScheme.error)
                            Spacer(modifier = Modifier.height(8.dp))
                            Text(text = errorMsg!!, color = MaterialTheme.colorScheme.onErrorContainer, style = MaterialTheme.typography.bodyMedium)
                            Spacer(modifier = Modifier.height(8.dp))
                            Button(onClick = {
                                isLoading = true
                                errorMsg = null
                            }) {
                                Text("Retry")
                            }
                        }
                    }
                }
            } else if (pairedDevices.isEmpty()) {
                item {
                    Card(
                        modifier = Modifier
                            .fillMaxWidth()
                            .border(1.dp, MaterialTheme.colorScheme.outline, RoundedCornerShape(14.dp)),
                        shape = RoundedCornerShape(14.dp),
                        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface)
                    ) {
                        Column(
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(24.dp),
                            horizontalAlignment = Alignment.CenterHorizontally
                        ) {
                            Surface(
                                shape = CircleShape,
                                color = MaterialTheme.colorScheme.surfaceVariant,
                                modifier = Modifier.size(54.dp)
                            ) {
                                Box(contentAlignment = Alignment.Center) {
                                    Icon(
                                        imageVector = Icons.Default.Devices,
                                        contentDescription = null,
                                        modifier = Modifier.size(26.dp),
                                        tint = MaterialTheme.colorScheme.primary
                                    )
                                }
                            }
                            Spacer(modifier = Modifier.height(12.dp))
                            Text(
                                text = "No devices paired yet",
                                style = MaterialTheme.typography.titleMedium,
                                color = MaterialTheme.colorScheme.onSurface
                            )
                            Spacer(modifier = Modifier.height(6.dp))
                            Text(
                                text = "Install CDUS on your laptop or desktop and scan the pairing QR code to sync clipboard and files directly.",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                textAlign = androidx.compose.ui.text.style.TextAlign.Center
                            )
                        }
                    }
                }
            } else {
                items(pairedDevices) { device ->
                    PairedDeviceItem(
                        device = device,
                        connectionState = connectionStates[device.nodeId],
                        isDeveloperMode = isDeveloperMode,
                        onUnpairClick = { unpairDevice(device.nodeId) },
                        onSendFileClick = {
                            selectedDeviceForFile = device.nodeId
                            filePickerLauncher.launch("*/*")
                        },
                        onReconnectClick = {
                            triggerActualConnect(device.nodeId)
                        },
                        onDisconnectClick = {
                            connectionStates[device.nodeId] = DeviceConnectionState(ConnectionStatus.OFFLINE, null, 0)
                            try {
                                uniffi.cdus_ffi.disconnectDevice(device.nodeId)
                            } catch (e: Exception) {
                                Logger.e("Error disconnecting device: ${e.message}")
                            }
                            android.widget.Toast.makeText(context, "Disconnected from ${UIUtils.formatDeviceLabel(device.label)}", android.widget.Toast.LENGTH_SHORT).show()
                        },
                        onBenchmarkClick = {
                            startBenchmark(device.nodeId)
                            android.widget.Toast.makeText(context, "Starting 1GB Benchmark...", android.widget.Toast.LENGTH_LONG).show()
                        }
                    )
                }
            }

            // Section 3: Discovered Devices on LAN
            if (isScanning || discoveredDevices.isNotEmpty()) {
                val filteredDiscovered = discoveredDevices.filter { d ->
                    pairedDevices.none { p -> p.nodeId == d.nodeId }
                }

                item {
                    Spacer(modifier = Modifier.height(8.dp))
                    Text(
                        text = "Discovered on Local Network",
                        style = MaterialTheme.typography.titleMedium,
                        color = MaterialTheme.colorScheme.onBackground
                    )
                }

                if (isScanning && filteredDiscovered.isEmpty()) {
                    item {
                        Surface(
                            modifier = Modifier
                                .fillMaxWidth()
                                .border(1.dp, MaterialTheme.colorScheme.outline, RoundedCornerShape(12.dp)),
                            shape = RoundedCornerShape(12.dp),
                            color = MaterialTheme.colorScheme.surfaceVariant
                        ) {
                            Row(
                                modifier = Modifier.padding(16.dp),
                                verticalAlignment = Alignment.CenterVertically
                            ) {
                                CircularProgressIndicator(
                                    modifier = Modifier.size(20.dp),
                                    strokeWidth = 2.dp,
                                    color = MaterialTheme.colorScheme.primary
                                )
                                Spacer(modifier = Modifier.width(12.dp))
                                Text(
                                    text = "Listening for mDNS beacons on LAN...",
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant
                                )
                            }
                        }
                    }
                } else {
                    items(filteredDiscovered) { device ->
                        DeviceListItem(
                            device = device,
                            onConnectClick = { initiatePairing(device.nodeId) }
                        )
                    }
                }
            }
        }
    }
}

@Composable
fun ConnectionPathBadge(transport: String) {
    val isLan = transport == "LAN"
    Surface(
        color = if (isLan) MaterialTheme.colorScheme.secondaryContainer else MaterialTheme.colorScheme.tertiaryContainer,
        shape = RoundedCornerShape(4.dp),
        modifier = Modifier.padding(start = 6.dp)
    ) {
        Text(
            text = if (isLan) "DIRECT LAN" else "RELAY",
            color = if (isLan) MaterialTheme.colorScheme.onSecondaryContainer else MaterialTheme.colorScheme.onTertiaryContainer,
            style = MaterialTheme.typography.labelSmall,
            modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp),
            fontWeight = FontWeight.Bold
        )
    }
}

@Composable
fun PairedDeviceItem(
    device: PairedDevice, 
    connectionState: DeviceConnectionState?,
    isDeveloperMode: Boolean = false,
    onUnpairClick: () -> Unit, 
    onSendFileClick: () -> Unit,
    onReconnectClick: () -> Unit,
    onDisconnectClick: () -> Unit,
    onBenchmarkClick: () -> Unit = {}
) {
    val status = connectionState?.status ?: if (device.isOnline) ConnectionStatus.ONLINE else ConnectionStatus.OFFLINE
    val transport = connectionState?.transport ?: if (device.isOnline) "LAN" else null
    val countdown = connectionState?.countdown ?: 0

    val isOnline = status == ConnectionStatus.ONLINE
    val isConnecting = status == ConnectionStatus.CONNECTING
    val isReconnecting = status == ConnectionStatus.RECONNECTING

    val statusText = when (status) {
        ConnectionStatus.ONLINE -> "Online"
        ConnectionStatus.CONNECTING -> "Connecting..."
        ConnectionStatus.RECONNECTING -> "Reconnecting in ${countdown}s..."
        ConnectionStatus.OFFLINE -> "Offline"
    }

    val infiniteTransition = rememberInfiniteTransition(label = "pulse")
    val alpha by infiniteTransition.animateFloat(
        initialValue = 0.4f,
        targetValue = 1f,
        animationSpec = infiniteRepeatable(
            animation = tween(durationMillis = if (isConnecting) 600 else 1000, easing = LinearEasing),
            repeatMode = RepeatMode.Reverse
        ),
        label = "alpha"
    )

    val dotColor = when (status) {
        ConnectionStatus.ONLINE -> MaterialTheme.colorScheme.secondary
        ConnectionStatus.CONNECTING -> MaterialTheme.colorScheme.primary
        ConnectionStatus.RECONNECTING -> MaterialTheme.colorScheme.tertiary
        ConnectionStatus.OFFLINE -> MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.5f)
    }

    val dotModifier = Modifier
        .size(8.dp)
        .let { modifier ->
            if (isConnecting || isReconnecting) {
                modifier.graphicsLayer { this.alpha = alpha }
            } else {
                modifier
            }
        }

    Card(
        modifier = Modifier
            .fillMaxWidth()
            .border(1.dp, MaterialTheme.colorScheme.outline, RoundedCornerShape(12.dp)),
        shape = RoundedCornerShape(12.dp),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface)
    ) {
        Column {
            Row(
                modifier = Modifier
                    .padding(14.dp)
                    .fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceBetween
            ) {
                Row(
                    modifier = Modifier.weight(1f),
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Surface(
                        shape = RoundedCornerShape(10.dp),
                        color = MaterialTheme.colorScheme.surfaceVariant,
                        modifier = Modifier.size(40.dp)
                    ) {
                        Box(contentAlignment = Alignment.Center) {
                            Icon(
                                imageVector = Icons.Default.Computer,
                                contentDescription = null,
                                modifier = Modifier.size(22.dp),
                                tint = if (isOnline) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant
                            )
                        }
                    }
                    Spacer(modifier = Modifier.width(12.dp))
                    Column(modifier = Modifier.weight(1f)) {
                        Text(
                            text = UIUtils.formatDeviceLabel(device.label),
                            style = MaterialTheme.typography.titleMedium,
                            color = MaterialTheme.colorScheme.onSurface,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis
                        )
                        Spacer(modifier = Modifier.height(2.dp))
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Surface(
                                modifier = dotModifier,
                                shape = CircleShape,
                                color = dotColor
                            ) {}
                            Spacer(modifier = Modifier.width(6.dp))
                            Text(
                                text = statusText,
                                style = MaterialTheme.typography.bodySmall,
                                color = if (isOnline) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant
                            )
                            
                            if (isOnline && transport != null) {
                                ConnectionPathBadge(transport)
                            }
                        }
                    }
                }

                Row(verticalAlignment = Alignment.CenterVertically) {
                    if (isOnline) {
                        Button(
                            onClick = onSendFileClick,
                            shape = RoundedCornerShape(8.dp),
                            contentPadding = PaddingValues(horizontal = 12.dp, vertical = 6.dp),
                            modifier = Modifier.height(36.dp)
                        ) {
                            Icon(Icons.Default.Send, contentDescription = null, modifier = Modifier.size(14.dp))
                            Spacer(modifier = Modifier.width(6.dp))
                            Text("Send", style = MaterialTheme.typography.labelMedium)
                        }
                    } else {
                        OutlinedButton(
                            onClick = onReconnectClick,
                            enabled = !isConnecting,
                            shape = RoundedCornerShape(8.dp),
                            contentPadding = PaddingValues(horizontal = 10.dp, vertical = 6.dp),
                            modifier = Modifier.height(36.dp)
                        ) {
                            Text(
                                if (isConnecting) "Connecting..." else "Reconnect",
                                style = MaterialTheme.typography.labelMedium
                            )
                        }
                    }
                    
                    var showMenu by remember { mutableStateOf(false) }
                    Box {
                        IconButton(onClick = { showMenu = true }) {
                            Icon(
                                imageVector = Icons.Default.MoreVert,
                                contentDescription = "Device Options",
                                tint = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                        }
                        DropdownMenu(
                            expanded = showMenu,
                            onDismissRequest = { showMenu = false }
                        ) {
                            if (isOnline) {
                                DropdownMenuItem(
                                    text = { Text("Disconnect") },
                                    onClick = {
                                        showMenu = false
                                        onDisconnectClick()
                                    }
                                )
                            }
                            DropdownMenuItem(
                                text = { Text("Unpair", color = MaterialTheme.colorScheme.error) },
                                onClick = {
                                    showMenu = false
                                    onUnpairClick()
                                }
                            )
                        }
                    }
                }
            }
            
            if (isDeveloperMode && isOnline) {
                Divider(
                    modifier = Modifier.padding(horizontal = 14.dp),
                    thickness = 0.5.dp,
                    color = MaterialTheme.colorScheme.outline
                )
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 14.dp, vertical = 4.dp),
                    horizontalArrangement = Arrangement.End
                ) {
                    TextButton(
                        onClick = onBenchmarkClick,
                        colors = ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.tertiary)
                    ) {
                        Text("Run 1GB Benchmark", style = MaterialTheme.typography.labelMedium)
                    }
                }
            }
        }
    }
}

@Composable
fun DeviceListItem(device: DiscoveredDevice, onConnectClick: () -> Unit) {
    Card(
        modifier = Modifier
            .fillMaxWidth()
            .border(1.dp, MaterialTheme.colorScheme.outline, RoundedCornerShape(12.dp)),
        shape = RoundedCornerShape(12.dp),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface)
    ) {
        Row(
            modifier = Modifier
                .padding(14.dp)
                .fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween
        ) {
            Row(
                modifier = Modifier.weight(1f),
                verticalAlignment = Alignment.CenterVertically
            ) {
                Surface(
                    shape = RoundedCornerShape(10.dp),
                    color = MaterialTheme.colorScheme.surfaceVariant,
                    modifier = Modifier.size(38.dp)
                ) {
                    Box(contentAlignment = Alignment.Center) {
                        Icon(
                            imageVector = if (device.os == "Android") Icons.Default.Smartphone else Icons.Default.Computer,
                            contentDescription = null,
                            modifier = Modifier.size(20.dp),
                            tint = MaterialTheme.colorScheme.primary
                        )
                    }
                }
                Spacer(modifier = Modifier.width(12.dp))
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        text = UIUtils.formatDeviceLabel(device.label),
                        style = MaterialTheme.typography.titleMedium,
                        color = MaterialTheme.colorScheme.onSurface,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis
                    )
                    Text(
                        text = "#${device.nodeId.take(8)} • ${device.os}",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            }
            Button(
                onClick = onConnectClick,
                shape = RoundedCornerShape(8.dp),
                contentPadding = PaddingValues(horizontal = 14.dp, vertical = 6.dp),
                modifier = Modifier.height(36.dp)
            ) {
                Text("Pair", style = MaterialTheme.typography.labelMedium)
            }
        }
    }
}


@Composable
fun PairingDialog(
    status: PairingStatus, 
    onDismiss: () -> Unit, 
    onConfirm: () -> Unit,
    onDecline: () -> Unit
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(text = "Pair with ${UIUtils.formatDeviceLabel(status.remoteLabel)}") },
        text = {
            Column(horizontalAlignment = Alignment.CenterHorizontally, modifier = Modifier.fillMaxWidth()) {
                Text(text = "Verify this PIN matches the other device:")
                Spacer(modifier = Modifier.height(16.dp))
                Text(
                    text = status.pin,
                    style = MaterialTheme.typography.displayMedium,
                    color = MaterialTheme.colorScheme.primary
                )
            }
        },
        confirmButton = {
            TextButton(onClick = onConfirm) {
                Text("Confirm")
            }
        },
        dismissButton = {
            TextButton(onClick = onDecline) {
                Text("Decline")
            }
        }
    )
}

@Composable
fun QrPairingDialog(onDismiss: () -> Unit) {
    val payload = remember { getQrPairingPayload() }
    val qrBitmap = remember(payload) {
        if (payload.isNotEmpty()) {
            val writer = QRCodeWriter()
            val bitMatrix = writer.encode(payload, BarcodeFormat.QR_CODE, 512, 512)
            val width = bitMatrix.width
            val height = bitMatrix.height
            val bitmap = Bitmap.createBitmap(width, height, Bitmap.Config.RGB_565)
            for (x in 0 until width) {
                for (y in 0 until height) {
                    bitmap.setPixel(x, y, if (bitMatrix.get(x, y)) AndroidColor.BLACK else AndroidColor.WHITE)
                }
            }
            bitmap.asImageBitmap()
        } else {
            null
        }
    }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("My Pairing QR") },
        text = {
            Column(horizontalAlignment = Alignment.CenterHorizontally, modifier = Modifier.fillMaxWidth()) {
                if (qrBitmap != null) {
                    Image(
                        bitmap = qrBitmap,
                        contentDescription = "Pairing QR Code",
                        modifier = Modifier.size(250.dp)
                    )
                    Spacer(modifier = Modifier.height(8.dp))
                    Text("Scan this from another device", style = MaterialTheme.typography.bodySmall)
                } else {
                    CircularProgressIndicator()
                }
            }
        },
        confirmButton = {
            TextButton(onClick = onDismiss) {
                Text("Close")
            }
        }
    )
}

@androidx.annotation.OptIn(androidx.camera.core.ExperimentalGetImage::class)
@Composable
fun QrScannerDialog(onDismiss: () -> Unit, onQrScanned: (String) -> Unit) {
    val context = LocalContext.current
    val executor = remember { Executors.newSingleThreadExecutor() }
    val scanner = remember { BarcodeScanning.getClient() }
    
    // Create a local lifecycle owner for this dialog to ensure CameraX shuts down correctly
    val lifecycleOwner = remember {
        object : androidx.lifecycle.LifecycleOwner {
            private val lifecycleRegistry = androidx.lifecycle.LifecycleRegistry(this)
            init {
                lifecycleRegistry.handleLifecycleEvent(androidx.lifecycle.Lifecycle.Event.ON_CREATE)
                lifecycleRegistry.handleLifecycleEvent(androidx.lifecycle.Lifecycle.Event.ON_START)
                lifecycleRegistry.handleLifecycleEvent(androidx.lifecycle.Lifecycle.Event.ON_RESUME)
            }
            override val lifecycle: androidx.lifecycle.Lifecycle = lifecycleRegistry
            fun destroy() {
                lifecycleRegistry.handleLifecycleEvent(androidx.lifecycle.Lifecycle.Event.ON_PAUSE)
                lifecycleRegistry.handleLifecycleEvent(androidx.lifecycle.Lifecycle.Event.ON_STOP)
                lifecycleRegistry.handleLifecycleEvent(androidx.lifecycle.Lifecycle.Event.ON_DESTROY)
            }
        }
    }

    var isScanned by remember { mutableStateOf(false) }
    val cameraProviderState = remember { mutableStateOf<ProcessCameraProvider?>(null) }
    val imageAnalysisState = remember { mutableStateOf<ImageAnalysis?>(null) }

    DisposableEffect(Unit) {
        onDispose {
            Logger.i("QrScannerDialog: Disposing resources")
            isScanned = true
            
            // Aggressively stop the analyzer and unbind the camera
            imageAnalysisState.value?.clearAnalyzer()
            cameraProviderState.value?.unbindAll()
            
            lifecycleOwner.destroy()
            executor.shutdownNow()
            try {
                scanner.close()
            } catch (e: Exception) {
                Logger.e("Error closing scanner: ${e.message}")
            }
        }
    }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Scan QR Code") },
        text = {
            Box(modifier = Modifier.size(300.dp).clip(MaterialTheme.shapes.medium)) {
                AndroidView(
                    factory = { ctx ->
                        val previewView = PreviewView(ctx).apply {
                            scaleType = PreviewView.ScaleType.FILL_CENTER
                        }
                        val cameraProviderFuture = ProcessCameraProvider.getInstance(ctx)
                        cameraProviderFuture.addListener({
                            if (isScanned || lifecycleOwner.lifecycle.currentState == androidx.lifecycle.Lifecycle.State.DESTROYED) return@addListener
                            
                            val cameraProvider = try {
                                cameraProviderFuture.get()
                            } catch (e: Exception) {
                                Logger.e("Failed to get camera provider: ${e.message}")
                                return@addListener
                            }
                            cameraProviderState.value = cameraProvider

                            val preview = Preview.Builder().build().also {
                                it.setSurfaceProvider(previewView.surfaceProvider)
                            }

                            val imageAnalysis = ImageAnalysis.Builder()
                                .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                                .build()
                            imageAnalysisState.value = imageAnalysis

                            imageAnalysis.setAnalyzer(executor) { imageProxy ->
                                if (isScanned) {
                                    imageProxy.close()
                                    return@setAnalyzer
                                }

                                try {
                                    val mediaImage = imageProxy.image
                                    if (mediaImage != null) {
                                        val image = InputImage.fromMediaImage(mediaImage, imageProxy.imageInfo.rotationDegrees)
                                        // Use a timeout to prevent hanging the executor thread indefinitely
                                        val barcodes = Tasks.await(scanner.process(image), 1, TimeUnit.SECONDS)
                                        
                                        if (isScanned) return@setAnalyzer
                                        
                                        for (barcode in barcodes) {
                                            val rawValue = barcode.rawValue
                                            if (rawValue != null && rawValue.startsWith("cdus://pair")) {
                                                isScanned = true
                                                Logger.i("QR Scanned successfully")
                                                android.os.Handler(android.os.Looper.getMainLooper()).post {
                                                    onQrScanned(rawValue)
                                                }
                                                break
                                            }
                                        }
                                    }
                                } catch (e: Exception) {
                                    if (!isScanned) {
                                        Logger.e("QR scanning error: ${e.message}")
                                    }
                                } finally {
                                    imageProxy.close()
                                }
                            }

                            val cameraSelector = CameraSelector.DEFAULT_BACK_CAMERA
                            try {
                                cameraProvider.unbindAll()
                                cameraProvider.bindToLifecycle(
                                    lifecycleOwner,
                                    cameraSelector,
                                    preview,
                                    imageAnalysis
                                )
                            } catch (e: Exception) {
                                Logger.e("Camera binding failed: ${e.message}")
                            }
                        }, ContextCompat.getMainExecutor(ctx))
                        previewView
                    },
                    modifier = Modifier.fillMaxSize()
                )
            }
        },
        confirmButton = {},
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text("Cancel")
            }
        }
    )
}
