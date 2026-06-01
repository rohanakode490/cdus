package io.cdus.app.ui.screens

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Computer
import androidx.compose.material.icons.filled.Smartphone
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
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
import io.cdus.app.utils.FileUtils
import io.cdus.app.utils.UIUtils
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.ui.platform.LocalContext

@Composable
fun DevicesScreen() {
    var isScanning by remember { mutableStateOf(false) }
    var discoveredDevices by remember { mutableStateOf<List<DiscoveredDevice>>(emptyList()) }
    var pairedDevices by remember { mutableStateOf<List<PairedDevice>>(emptyList()) }
    var pairingStatus by remember { mutableStateOf<PairingStatus?>(null) }
    var isDeveloperMode by remember { mutableStateOf(false) }
    
    val context = LocalContext.current
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
            pairingStatus = getPairingStatus()
            val devices = getPairedDevices()
            pairedDevices = devices
            isDeveloperMode = sharedPref.getBoolean("developer_mode", false)
            io.cdus.app.data.DeviceManager.updateLabels(devices)
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

        if (pairedDevices.isEmpty()) {
            Text(
                text = "No devices paired yet.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.outline,
                modifier = Modifier.padding(bottom = 16.dp)
            )
        } else {
            for (device in pairedDevices) {
                PairedDeviceItem(
                    device = device,
                    isDeveloperMode = isDeveloperMode,
                    onUnpairClick = { unpairDevice(device.nodeId) },
                    onSendFileClick = {
                        selectedDeviceForFile = device.nodeId
                        filePickerLauncher.launch("*/*")
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

        Button(
            onClick = { isScanning = !isScanning },
            modifier = Modifier.fillMaxWidth()
        ) {
            if (isScanning) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    CircularProgressIndicator(
                        modifier = Modifier.size(16.dp),
                        strokeWidth = 2.dp,
                        color = MaterialTheme.colorScheme.onPrimary
                    )
                    Spacer(modifier = Modifier.width(8.dp))
                    Text("Stop Scan")
                }
            } else {
                Text("Start Scan")
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
fun PairedDeviceItem(
    device: PairedDevice, 
    isDeveloperMode: Boolean = false,
    onUnpairClick: () -> Unit, 
    onSendFileClick: () -> Unit,
    onBenchmarkClick: () -> Unit = {}
) {
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
                                modifier = Modifier.size(8.dp),
                                shape = androidx.compose.foundation.shape.CircleShape,
                                color = if (device.isOnline) androidx.compose.ui.graphics.Color.Green else androidx.compose.ui.graphics.Color.Gray
                            ) {}
                            Spacer(modifier = Modifier.width(6.dp))
                            Text(
                                text = if (device.isOnline) "Online" else "Offline",
                                style = MaterialTheme.typography.bodySmall,
                                color = if (device.isOnline) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.outline
                            )
                            Text(text = " • #${device.nodeId.take(8)}", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.secondary)
                        }
                    }
                }
                Row {
                    if (device.isOnline) {
                        TextButton(onClick = onSendFileClick) {
                            Text("Send File")
                        }
                    } else {
                        TextButton(onClick = { initiatePairing(device.nodeId) }) {
                            Text("Connect")
                        }
                    }
                    TextButton(onClick = onUnpairClick) {
                        Text("Unpair", color = MaterialTheme.colorScheme.error)
                    }
                }
            }
            
            if (isDeveloperMode && device.isOnline) {
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
