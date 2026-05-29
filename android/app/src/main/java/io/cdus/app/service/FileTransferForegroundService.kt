package io.cdus.app.service

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.os.IBinder
import androidx.core.app.NotificationCompat
import io.cdus.app.utils.Logger

class FileTransferForegroundService : Service() {

    companion object {
        const val CHANNEL_ID = "file_transfer_active_channel"
        const val NOTIFICATION_ID = 3
        const val ACTION_START = "ACTION_START_TRANSFER"
        const val ACTION_STOP = "ACTION_STOP_TRANSFER"
        const val EXTRA_TRANSFER_ID = "transfer_id"
    }

    private var activeTransfers = mutableSetOf<String>()

    override fun onCreate() {
        super.onCreate()
        createNotificationChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val action = intent?.action
        val transferId = intent?.getStringExtra(EXTRA_TRANSFER_ID) ?: ""

        when (action) {
            ACTION_START -> {
                activeTransfers.add(transferId)
                updateNotification()
            }
            ACTION_STOP -> {
                activeTransfers.remove(transferId)
                if (activeTransfers.isEmpty()) {
                    stopForeground(STOP_FOREGROUND_REMOVE)
                    stopSelf()
                } else {
                    updateNotification()
                }
            }
        }

        return START_NOT_STICKY
    }

    private fun updateNotification() {
        if (activeTransfers.isEmpty()) return

        val notification = NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(android.R.drawable.stat_sys_download)
            .setContentTitle("CDUS File Transfer")
            .setContentText("${activeTransfers.size} active transfer(s)")
            .setOngoing(true)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .build()

        startForeground(NOTIFICATION_ID, notification)
    }

    override fun onDestroy() {
        super.onDestroy()
        Logger.i("FileTransferForegroundService destroyed")
    }

    override fun onBind(intent: Intent?): IBinder? = null

    private fun createNotificationChannel() {
        val manager = getSystemService(NotificationManager::class.java)
        val serviceChannel = NotificationChannel(
            CHANNEL_ID,
            "Active File Transfers",
            NotificationManager.IMPORTANCE_LOW
        )
        manager.createNotificationChannel(serviceChannel)
    }
}
