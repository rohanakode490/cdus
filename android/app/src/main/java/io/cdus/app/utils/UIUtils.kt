package io.cdus.app.utils

import org.json.JSONObject

object UIUtils {
    /**
     * Attempts to extract a readable label from a potentially JSON handshake payload.
     * If the string is not valid JSON or doesn't contain a 'label' field, returns the original string.
     */
    fun formatDeviceLabel(raw: String): String {
        return try {
            if (raw.startsWith("{")) {
                val json = JSONObject(raw)
                json.optString("label", raw)
            } else {
                raw
            }
        } catch (e: Exception) {
            raw
        }
    }
}
