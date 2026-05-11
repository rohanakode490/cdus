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

@Composable
fun DevicesScreen() {
    var isScanning by remember { mutableStateOf(false) }
    var devices by remember { mutableStateOf(emptyList<DiscoveredDevice>()) }
    var pairingStatus by remember { mutableStateOf<PairingStatus?>(null) }
    val scope = rememberCoroutineScope()

    LaunchedEffect(isScanning) {
        if (isScanning) {
            clearDiscoveredDevices()
            devices = emptyList()
            startDiscovery()
            while (isActive) {
                devices = getDiscoveredDevices()
                delay(1000)
            }
        } else {
            stopDiscovery()
            // Don't clear devices here, keep them visible
        }
    }

    LaunchedEffect(Unit) {
        while (isActive) {
            pairingStatus = getPairingStatus()
            delay(1000)
        }
    }

    if (pairingStatus != null && pairingStatus!!.active) {
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
            .padding(16.dp),
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        Text(
            text = "Device Discovery",
            style = MaterialTheme.typography.headlineMedium,
            modifier = Modifier.padding(bottom = 16.dp)
        )

        Button(
            onClick = { isScanning = !isScanning },
            modifier = Modifier.fillMaxWidth()
        ) {
            Text(if (isScanning) "Stop Scan" else "Start Scan")
        }

        Spacer(modifier = Modifier.height(16.dp))

        if (isScanning && devices.isEmpty()) {
            CircularProgressIndicator()
            Spacer(modifier = Modifier.height(8.dp))
            Text("Scanning for devices...")
        } else if (!isScanning && devices.isEmpty()) {
            Box(modifier = Modifier.weight(1f), contentAlignment = Alignment.Center) {
                Text("No devices found. Tap Start Scan to search.")
            }
        } else {
            LazyColumn(modifier = Modifier.weight(1f)) {
                items(devices) { device ->
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
                    Text(text = device.label, style = MaterialTheme.typography.bodyLarge)
                    Text(text = "${device.os} • ${device.ip}", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.secondary)
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
        title = { Text(text = "Pair with ${status.remoteLabel}") },
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
