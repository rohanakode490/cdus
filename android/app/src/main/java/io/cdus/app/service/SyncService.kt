package io.cdus.app.service

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.IBinder
import android.util.Log
import androidx.core.app.NotificationCompat
import uniffi.cdus_ffi.broadcastClipboard
import uniffi.cdus_ffi.setClipboardListener
import uniffi.cdus_ffi.ClipboardListener

class SyncService : Service(), ClipboardListener {

    private val CHANNEL_ID = "sync_channel"
    private val NOTIFICATION_ID = 1
    private lateinit var clipboardManager: ClipboardManager
    private var lastClipboardContent: String? = null

    override fun onCreate() {
        super.onCreate()
        createNotificationChannel()
        clipboardManager = getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        setClipboardListener(this)
        Log.i("CDUS", "SyncService created and remote listener added")
    }

    override fun onClipboardUpdate(content: String, source: String) {
        Log.i("CDUS", "Received remote clipboard update from $source: $content")
        if (content != lastClipboardContent) {
            lastClipboardContent = content
            // ClipboardManager.setPrimaryClip must be called on the main thread
            android.os.Handler(android.os.Looper.getMainLooper()).post {
                try {
                    val clip = android.content.ClipData.newPlainText("CDUS Remote", content)
                    clipboardManager.setPrimaryClip(clip)
                    Log.d("CDUS", "Successfully set system clipboard from remote source")
                } catch (e: Exception) {
                    Log.e("CDUS", "Failed to set system clipboard: ${e.message}")
                }
            }
        } else {
            Log.d("CDUS", "Ignored remote update as it matches current local state")
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val notification = NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(android.R.drawable.ic_popup_sync)
            .setContentTitle("CDUS Sync Active")
            .setContentText("Syncing clipboard across devices...")
            .setOngoing(true)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .build()

        startForeground(NOTIFICATION_ID, notification)

        return START_STICKY
    }

    override fun onDestroy() {
        super.onDestroy()
        Log.i("CDUS", "SyncService destroyed")
    }

    override fun onBind(intent: Intent?): IBinder? = null

    private fun createNotificationChannel() {
        val serviceChannel = NotificationChannel(
            CHANNEL_ID,
            "Sync Status",
            NotificationManager.IMPORTANCE_LOW
        )
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(serviceChannel)
    }
}
