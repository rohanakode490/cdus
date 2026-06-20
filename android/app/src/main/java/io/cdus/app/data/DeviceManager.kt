package io.cdus.app.data

import androidx.compose.runtime.mutableStateMapOf

object DeviceManager {
    // nodeId -> label
    val pairedDeviceLabels = mutableStateMapOf<String, String>()
    
    var isRelayConnected = androidx.compose.runtime.mutableStateOf(true)
    var relayError = androidx.compose.runtime.mutableStateOf<String?>(null)
    
    fun updateLabels(devices: List<uniffi.cdus_ffi.PairedDevice>) {
        devices.forEach { device ->
            pairedDeviceLabels[device.nodeId] = device.label
        }
    }

    fun getLabel(nodeId: String): String {
        return pairedDeviceLabels[nodeId] ?: nodeId.take(8)
    }
}
