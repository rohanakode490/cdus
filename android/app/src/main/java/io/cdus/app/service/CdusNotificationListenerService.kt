package io.cdus.app.service

import android.app.Notification
import android.content.Context
import android.content.SharedPreferences
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import io.cdus.app.utils.Logger
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import uniffi.cdus_ffi.sendNotificationMirror
import uniffi.cdus_ffi.sendNotificationDismiss

class CdusNotificationListenerService : NotificationListenerService() {

    companion object {
        @Volatile
        private var instance: CdusNotificationListenerService? = null

        fun getSharedInstance(): CdusNotificationListenerService? {
            return instance
        }

        fun cancelNotification(key: String) {
            val currentInstance = instance
            if (currentInstance != null) {
                try {
                    currentInstance.cancelAllNotifications() // Fallback or cancel specific? cancelNotification is API 21+
                    currentInstance.cancelNotification(key)
                    Logger.i("Successfully dismissed notification with key: $key")
                } catch (e: Exception) {
                    Logger.e("Failed to cancel notification: ${e.message}")
                }
            } else {
                Logger.w("CdusNotificationListenerService instance is not running, cannot dismiss: $key")
            }
        }
    }

    private lateinit var sharedPref: SharedPreferences

    override fun onCreate() {
        super.onCreate()
        instance = this
        sharedPref = getSharedPreferences("cdus_settings", Context.MODE_PRIVATE)
        Logger.i("CdusNotificationListenerService created")
    }

    override fun onDestroy() {
        super.onDestroy()
        if (instance === this) {
            instance = null
        }
        Logger.i("CdusNotificationListenerService destroyed")
    }

    override fun onNotificationPosted(sbn: StatusBarNotification?) {
        super.onNotificationPosted(sbn)
        if (sbn == null) return

        try {
            val globalEnabled = sharedPref.getBoolean("notification_sync_enabled", false)
            if (!globalEnabled) return

            val packageName = sbn.packageName
            // Do not sync our own app notifications to avoid loop
            if (packageName == packageName) { // Wait, packageName is sbn.packageName. context's package name is this.packageName
                if (packageName == this.packageName) {
                    return
                }
            }

            val appEnabled = sharedPref.getBoolean("notify_app_$packageName", true)
            if (!appEnabled) return

            val extras = sbn.notification.extras
            val title = extras.getCharSequence(Notification.EXTRA_TITLE)?.toString() ?: ""
            val text = extras.getCharSequence(Notification.EXTRA_TEXT)?.toString() ?: ""
            val timestamp = sbn.postTime

            // Get app label
            val pm = packageManager
            val appName = try {
                val ai = pm.getApplicationInfo(packageName, 0)
                pm.getApplicationLabel(ai).toString()
            } catch (e: Exception) {
                packageName
            }

            val key = sbn.key ?: ""

            // UniFFI call must run on Dispatchers.IO context
            CoroutineScope(Dispatchers.IO).launch {
                Logger.d("Mirroring notification: $appName, Title: $title, Key: $key")
                sendNotificationMirror(
                    key = key,
                    packageName = packageName,
                    appName = appName,
                    title = title,
                    text = text,
                    timestamp = timestamp.toULong()
                )
            }
        } catch (e: Exception) {
            Logger.e("Error processing notification post: ${e.message}")
        }
    }

    override fun onNotificationRemoved(sbn: StatusBarNotification?) {
        super.onNotificationRemoved(sbn)
        if (sbn == null) return

        try {
            val globalEnabled = sharedPref.getBoolean("notification_sync_enabled", false)
            if (!globalEnabled) return

            val key = sbn.key ?: ""

            CoroutineScope(Dispatchers.IO).launch {
                Logger.d("Notification removed locally, sending dismiss: $key")
                sendNotificationDismiss(key)
            }
        } catch (e: Exception) {
            Logger.e("Error processing notification remove: ${e.message}")
        }
    }
}
