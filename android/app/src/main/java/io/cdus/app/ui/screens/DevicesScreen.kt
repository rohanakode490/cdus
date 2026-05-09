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

data class Device(val id: String, val name: String, val os: String)

@Composable
fun DevicesScreen() {
    var isScanning by remember { mutableStateOf(false) }
    var mockDevices by remember { mutableStateOf(emptyList<Device>()) }
    var deviceToPair by remember { mutableStateOf<Device?>(null) }
    val scope = rememberCoroutineScope()

    LaunchedEffect(isScanning) {
        if (isScanning) {
            delay(2000) // Simulate discovery delay
            mockDevices = listOf(
                Device("1", "MacBook Pro", "macOS"),
                Device("2", "Linux Desktop", "Linux"),
                Device("3", "Pixel 7", "Android")
            )
        } else {
            mockDevices = emptyList()
        }
    }

    if (deviceToPair != null) {
        PairingDialog(
            device = deviceToPair!!,
            onDismiss = { deviceToPair = null },
            onConfirm = { deviceToPair = null }
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

        if (isScanning && mockDevices.isEmpty()) {
            CircularProgressIndicator()
            Spacer(modifier = Modifier.height(8.dp))
            Text("Scanning for devices...")
        } else if (!isScanning && mockDevices.isEmpty()) {
            Box(modifier = Modifier.weight(1f), contentAlignment = Alignment.Center) {
                Text("No devices found. Tap Start Scan to search.")
            }
        } else {
            LazyColumn(modifier = Modifier.weight(1f)) {
                items(mockDevices) { device ->
                    DeviceListItem(
                        device = device,
                        onConnectClick = { deviceToPair = device }
                    )
                }
            }
        }
    }
}

@Composable
fun DeviceListItem(device: Device, onConnectClick: () -> Unit) {
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
                    Text(text = device.name, style = MaterialTheme.typography.bodyLarge)
                    Text(text = device.os, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.secondary)
                }
            }
            Button(onClick = onConnectClick) {
                Text("Connect")
            }
        }
    }
}

@Composable
fun PairingDialog(device: Device, onDismiss: () -> Unit, onConfirm: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(text = "Pair with ${device.name}") },
        text = {
            Column(horizontalAlignment = Alignment.CenterHorizontally, modifier = Modifier.fillMaxWidth()) {
                Text(text = "Verify this PIN matches the other device:")
                Spacer(modifier = Modifier.height(16.dp))
                Text(
                    text = "1234",
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
            TextButton(onClick = onDismiss) {
                Text("Decline")
            }
        }
    )
}

