package io.cdus.app.ui.components

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import uniffi.cdus_ffi.PairedDevice
import uniffi.cdus_ffi.getPairedDevices
import io.cdus.app.utils.UIUtils

@Composable
fun DevicePickerDialog(
    onDeviceSelected: (String) -> Unit,
    onDismiss: () -> Unit
) {
    val pairedDevices = remember { getPairedDevices() }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Select Device to Send File") },
        text = {
            if (pairedDevices.isEmpty()) {
                Text("No paired devices found. Please pair a device first.")
            } else {
                LazyColumn {
                    items(pairedDevices) { device ->
                        ListItem(
                            headlineContent = { Text(UIUtils.formatDeviceLabel(device.label)) },
                            modifier = Modifier.clickable {
                                onDeviceSelected(device.nodeId)
                            }
                        )
                    }
                }
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
