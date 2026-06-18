package io.cdus.app.ui.screens

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Computer
import androidx.compose.material.icons.filled.Smartphone
import androidx.compose.material.icons.filled.MoreVert
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

data class AndroidMockDeviceState(
    val status: String, // "online", "offline", "reconnecting", "connecting"
    val transport: String?, // "LAN", "Relay", null
    val countdown: Int
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
    
    // --- Mock Reconnection States ---
    val mockStates = remember { mutableStateMapOf<String, AndroidMockDeviceState>() }
    val scope = rememberCoroutineScope()

    fun triggerMockConnect(deviceId: String, label: String) {
        mockStates[deviceId] = AndroidMockDeviceState("connecting", null, 0)
        scope.launch {
            delay(10000)
            val state = mockStates[deviceId]
            if (state != null && state.status == "connecting") {
                mockStates[deviceId] = AndroidMockDeviceState("offline", null, 0)
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
                
                // Initialize mock states for devices and sync with real connectivity status
                devices.forEach { device ->
                    val state = mockStates[device.nodeId]
                    if (device.isOnline) {
                        if (state == null || state.status != "online") {
                            mockStates[device.nodeId] = AndroidMockDeviceState("online", "LAN", 0)
                        }
                    } else {
                        if (state == null || state.status == "online") {
                            mockStates[device.nodeId] = AndroidMockDeviceState("offline", null, 0)
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
        Text(
            text = "Devices",
            style = MaterialTheme.typography.headlineMedium,
            modifier = Modifier.padding(bottom = 16.dp)
        )

        // Paired Devices Section
        Text(
            text = "Your Devices",
            style = MaterialTheme.typography.titleMedium,
            color = MaterialTheme.colorScheme.secondary,
            modifier = Modifier.padding(bottom = 8.dp)
        )

        if (isLoading) {
            Box(
                modifier = Modifier.fillMaxWidth().height(80.dp),
                contentAlignment = Alignment.Center
            ) {
                CircularProgressIndicator(modifier = Modifier.size(24.dp))
            }
        } else if (errorMsg != null) {
            Column(
                modifier = Modifier.fillMaxWidth().padding(vertical = 16.dp),
                horizontalAlignment = Alignment.CenterHorizontally
            ) {
                Text(text = errorMsg!!, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodyMedium)
                Spacer(modifier = Modifier.height(8.dp))
                Button(onClick = {
                    isLoading = true
                    errorMsg = null
                }) {
                    Text("Retry")
                }
            }
        } else if (pairedDevices.isEmpty()) {
            Text(
                text = "No devices paired yet. Start scanning to connect your first device.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.outline,
                modifier = Modifier.padding(bottom = 16.dp)
            )
        } else {
            for (device in pairedDevices) {
                PairedDeviceItem(
                    device = device,
                    mockState = mockStates[device.nodeId],
                    isDeveloperMode = isDeveloperMode,
                    onUnpairClick = { unpairDevice(device.nodeId) },
                    onSendFileClick = {
                        selectedDeviceForFile = device.nodeId
                        filePickerLauncher.launch("*/*")
                    },
                    onReconnectClick = {
                        triggerMockConnect(device.nodeId, device.label)
                        initiatePairing(device.nodeId)
                    },
                    onDisconnectClick = {
                        mockStates[device.nodeId] = AndroidMockDeviceState("offline", null, 0)
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

        Spacer(modifier = Modifier.height(24.dp))

        // Discovery Section
        Text(
            text = "Discovery",
            style = MaterialTheme.typography.titleMedium,
            color = MaterialTheme.colorScheme.secondary,
            modifier = Modifier.padding(bottom = 8.dp)
        )

        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            Button(
                onClick = { showQrDialog = true },
                modifier = Modifier.weight(1f)
            ) {
                Text("Show QR")
            }
            Button(
                onClick = { cameraPermissionLauncher.launch(android.Manifest.permission.CAMERA) },
                modifier = Modifier.weight(1f)
            ) {
                Text("Scan QR")
            }
            Button(
                onClick = { isScanning = !isScanning },
                modifier = Modifier.weight(1.5f)
            ) {
                if (isScanning) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        CircularProgressIndicator(
                            modifier = Modifier.size(16.dp),
                            strokeWidth = 2.dp,
                            color = MaterialTheme.colorScheme.onPrimary
                        )
                        Spacer(modifier = Modifier.width(8.dp))
                        Text("Stop")
                    }
                } else {
                    Text("Scan LAN")
                }
            }
        }

        Spacer(modifier = Modifier.height(16.dp))

        if (isScanning && discoveredDevices.isEmpty()) {
            Box(modifier = Modifier.fillMaxWidth().padding(16.dp), contentAlignment = Alignment.Center) {
                CircularProgressIndicator()
            }
        } else if (!isScanning && discoveredDevices.isEmpty()) {
            // Nothing to show
        } else {
            LazyColumn(modifier = Modifier.weight(1f)) {
                // Filter out already paired devices from discovered list
                val filteredDiscovered = discoveredDevices.filter { d ->
                    pairedDevices.none { p -> p.nodeId == d.nodeId }
                }
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

@Composable
fun ConnectionPathBadge(transport: String) {
    val containerColor = when (transport) {
        "LAN" -> androidx.compose.ui.graphics.Color(0xFFE8F5E9)
        "Relay" -> androidx.compose.ui.graphics.Color(0xFFFFF3E0)
        else -> androidx.compose.ui.graphics.Color(0xFFFFEBEE)
    }
    val contentColor = when (transport) {
        "LAN" -> androidx.compose.ui.graphics.Color(0xFF2E7D32)
        "Relay" -> androidx.compose.ui.graphics.Color(0xFFEF6C00)
        else -> androidx.compose.ui.graphics.Color(0xFFC62828)
    }
    Surface(
        color = containerColor,
        shape = androidx.compose.foundation.shape.RoundedCornerShape(4.dp),
        modifier = Modifier.padding(start = 6.dp)
    ) {
        Text(
            text = transport,
            color = contentColor,
            style = MaterialTheme.typography.labelSmall,
            modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp),
            fontWeight = androidx.compose.ui.text.font.FontWeight.Bold
        )
    }
}

@Composable
fun PairedDeviceItem(
    device: PairedDevice, 
    mockState: AndroidMockDeviceState?,
    isDeveloperMode: Boolean = false,
    onUnpairClick: () -> Unit, 
    onSendFileClick: () -> Unit,
    onReconnectClick: () -> Unit,
    onDisconnectClick: () -> Unit,
    onBenchmarkClick: () -> Unit = {}
) {
    val status = mockState?.status ?: if (device.isOnline) "online" else "offline"
    val transport = mockState?.transport ?: if (device.isOnline) "LAN" else null
    val countdown = mockState?.countdown ?: 0

    val isOnline = status == "online"
    val isConnecting = status == "connecting"
    val isReconnecting = status == "reconnecting"

    val statusText = when (status) {
        "online" -> "Online"
        "connecting" -> "Connecting..."
        "reconnecting" -> "Reconnecting in ${countdown}s..."
        else -> "Offline"
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
        "online" -> androidx.compose.ui.graphics.Color.Green
        "connecting" -> androidx.compose.ui.graphics.Color(0xFF2196F3)
        else -> androidx.compose.ui.graphics.Color.Gray
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
            .padding(vertical = 4.dp),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceVariant)
    ) {
        Column {
            Row(
                modifier = Modifier
                    .padding(12.dp)
                    .fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceBetween
            ) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Icon(
                        imageVector = Icons.Default.Computer,
                        contentDescription = null,
                        modifier = Modifier.size(20.dp),
                        tint = MaterialTheme.colorScheme.primary
                    )
                    Spacer(modifier = Modifier.width(12.dp))
                    Column {
                        Text(text = UIUtils.formatDeviceLabel(device.label), style = MaterialTheme.typography.bodyLarge)
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Surface(
                                modifier = dotModifier,
                                shape = androidx.compose.foundation.shape.CircleShape,
                                color = dotColor
                            ) {}
                            Spacer(modifier = Modifier.width(6.dp))
                            Text(
                                text = statusText,
                                style = MaterialTheme.typography.bodySmall,
                                color = if (isOnline) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.outline
                            )
                            
                            if (isOnline && transport != null) {
                                ConnectionPathBadge(transport)
                            }
                        }
                    }
                }
                Row(verticalAlignment = Alignment.CenterVertically) {
                    if (isOnline) {
                        TextButton(onClick = onSendFileClick) {
                            Text("Send File")
                        }
                    } else {
                        TextButton(
                            onClick = onReconnectClick,
                            enabled = !isConnecting
                        ) {
                            Text(if (isConnecting) "Connecting..." else "Reconnect Now")
                        }
                    }
                    
                    var showMenu by remember { mutableStateOf(false) }
                    Box {
                        IconButton(onClick = { showMenu = true }) {
                            Icon(
                                imageVector = Icons.Default.MoreVert,
                                contentDescription = "Device Options"
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
                Divider(modifier = Modifier.padding(horizontal = 12.dp), thickness = 0.5.dp, color = MaterialTheme.colorScheme.outlineVariant)
                Row(
                    modifier = Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 4.dp),
                    horizontalArrangement = Arrangement.End
                ) {
                    TextButton(
                        onClick = onBenchmarkClick,
                        colors = ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.tertiary)
                    ) {
                        Text("Run 1GB Benchmark")
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
            .padding(vertical = 4.dp),
        elevation = CardDefaults.cardElevation(defaultElevation = 2.dp)
    ) {
        Row(
            modifier = Modifier
                .padding(16.dp)
                .fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(
                    imageVector = if (device.os == "Android") Icons.Default.Smartphone else Icons.Default.Computer,
                    contentDescription = null,
                    modifier = Modifier.size(24.dp)
                )
                Spacer(modifier = Modifier.width(16.dp))
                Column {
                    Text(text = UIUtils.formatDeviceLabel(device.label), style = MaterialTheme.typography.bodyLarge)
                    Text(text = "#${device.nodeId.take(8)} • ${device.os}", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.secondary)
                }
            }
            Button(onClick = onConnectClick) {
                Text("Connect")
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
