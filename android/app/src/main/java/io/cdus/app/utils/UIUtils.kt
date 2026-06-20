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

    /**
     * Sanitizes raw error messages by removing URLs, IP addresses, and hash IDs to protect user privacy.
     */
    fun sanitizeErrorMessage(err: String?): String {
        if (err == null) return "Unknown error"
        var sanitized = err.replace(Regex("https?://[^\\s]+"), "[URL]")
        sanitized = sanitized.replace(Regex("wss?://[^\\s]+"), "[URL]")
        sanitized = sanitized.replace(Regex("\\b\\d{1,3}\\.\\d{1,3}\\.\\d{1,3}\\.\\d{1,3}(:\\d+)?\\b"), "[address]")
        sanitized = sanitized.replace(Regex("\\b[0-9a-fA-F]{32,}\\b"), "[id]")
        return sanitized
    }
}
