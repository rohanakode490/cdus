package io.cdus.app.service

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import uniffi.cdus_ffi.broadcastClipboard
import uniffi.cdus_ffi.setClipboardListener
import uniffi.cdus_ffi.ClipboardListener
import uniffi.cdus_ffi.FileTransferListener
import uniffi.cdus_ffi.setFileTransferListener
import uniffi.cdus_ffi.FileManifest
import uniffi.cdus_ffi.acceptFileTransfer
import uniffi.cdus_ffi.rejectFileTransfer
import io.cdus.app.data.FileTransferManager
import io.cdus.app.data.FileTransferInfo
import io.cdus.app.data.TransferStatus
import io.cdus.app.utils.FileUtils
import io.cdus.app.utils.UIUtils
import io.cdus.app.utils.Logger

class SyncService : Service(), ClipboardListener, FileTransferListener {

    private val CHANNEL_ID = "sync_channel"
    private val FILE_CHANNEL_ID = "file_transfer_channel"
    private val NOTIFICATION_ID = 1
    private val FILE_NOTIFICATION_ID = 2
    private lateinit var clipboardManager: ClipboardManager
    private var lastClipboardContent: String? = null

    override fun onCreate() {
        super.onCreate()
        createNotificationChannels()
        clipboardManager = getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        setClipboardListener(this)
        setFileTransferListener(this)
        Logger.i("SyncService created and remote listeners added")
    }

    override fun onClipboardUpdate(content: String, source: String) {
        Logger.i("Received remote clipboard update from $source: $content")
        if (content != lastClipboardContent) {
            lastClipboardContent = content
            android.os.Handler(android.os.Looper.getMainLooper()).post {
                try {
                    val clip = android.content.ClipData.newPlainText("CDUS Remote", content)
                    clipboardManager.setPrimaryClip(clip)
                    Logger.d("Successfully set system clipboard from remote source")
                } catch (e: Exception) {
                    Logger.e("Failed to set system clipboard: ${e.message}")
                }
            }
        }
    }

    override fun onIncomingRequest(nodeId: String, manifest: FileManifest) {
        Logger.i("Incoming file request from $nodeId: ${manifest.fileName}")
        
        FileTransferManager.updateTransfer(
            FileTransferInfo(
                fileHash = manifest.fileHash,
                fileName = manifest.fileName,
                progress = 0f,
                status = TransferStatus.INCOMING
            )
        )

        // Show notification with Accept/Decline actions
        val acceptIntent = Intent(this, FileActionReceiver::class.java).apply {
            action = "ACCEPT"
            putExtra("file_hash", manifest.fileHash)
        }
        val acceptPendingIntent = android.app.PendingIntent.getBroadcast(this, 0, acceptIntent, android.app.PendingIntent.FLAG_IMMUTABLE)

        val declineIntent = Intent(this, FileActionReceiver::class.java).apply {
            action = "DECLINE"
            putExtra("file_hash", manifest.fileHash)
        }
        val declinePendingIntent = android.app.PendingIntent.getBroadcast(this, 1, declineIntent, android.app.PendingIntent.FLAG_IMMUTABLE)

        val notification = NotificationCompat.Builder(this, FILE_CHANNEL_ID)
            .setSmallIcon(android.R.drawable.stat_sys_download)
            .setContentTitle("Incoming File")
            .setContentText("${manifest.fileName} from ${UIUtils.formatDeviceLabel(nodeId)}")
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .addAction(android.R.drawable.checkbox_on_background, "Accept", acceptPendingIntent)
            .addAction(android.R.drawable.ic_menu_close_clear_cancel, "Decline", declinePendingIntent)
            .setAutoCancel(true)
            .build()

        val notificationManager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        notificationManager.notify(FILE_NOTIFICATION_ID, notification)
    }

    override fun onTransferProgress(fileHash: String, progress: Float) {
        Logger.d("Transfer progress for $fileHash: $progress%")
        FileTransferManager.updateProgress(fileHash, progress)

        val notification = NotificationCompat.Builder(this, FILE_CHANNEL_ID)
            .setSmallIcon(android.R.drawable.stat_sys_download)
            .setContentTitle("Downloading File")
            .setContentText("Transfer in progress...")
            .setProgress(100, progress.toInt(), false)
            .setOngoing(true)
            .build()

        val notificationManager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        notificationManager.notify(FILE_NOTIFICATION_ID, notification)
    }

    override fun onTransferComplete(fileHash: String) {
        Logger.i("Transfer complete: $fileHash")
        FileTransferManager.markComplete(fileHash)
        
        val info = FileTransferManager.transfers[fileHash]
        if (info != null) {
            val internalFile = java.io.File(filesDir, info.fileName)
            if (internalFile.exists()) {
                FileUtils.saveFileToDownloads(this, internalFile)
            }
        }

        val notification = NotificationCompat.Builder(this, FILE_CHANNEL_ID)
            .setSmallIcon(android.R.drawable.stat_sys_download_done)
            .setContentTitle("Transfer Complete")
            .setContentText("File saved successfully.")
            .setPriority(NotificationCompat.PRIORITY_DEFAULT)
            .build()

        val notificationManager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        notificationManager.notify(FILE_NOTIFICATION_ID, notification)
    }

    override fun onTransferError(fileHash: String, error: String) {
        Logger.e("Transfer error for $fileHash: $error")
        FileTransferManager.markError(fileHash, error)

        val notification = NotificationCompat.Builder(this, FILE_CHANNEL_ID)
            .setSmallIcon(android.R.drawable.ic_dialog_alert)
            .setContentTitle("Transfer Failed")
            .setContentText(error)
            .setPriority(NotificationCompat.PRIORITY_DEFAULT)
            .build()

        val notificationManager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        notificationManager.notify(FILE_NOTIFICATION_ID, notification)
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
        Logger.i("SyncService destroyed")
    }

    override fun onBind(intent: Intent?): IBinder? = null

    private fun createNotificationChannels() {
        val manager = getSystemService(NotificationManager::class.java)
        
        val serviceChannel = NotificationChannel(
            CHANNEL_ID,
            "Sync Status",
            NotificationManager.IMPORTANCE_LOW
        )
        manager.createNotificationChannel(serviceChannel)

        val fileChannel = NotificationChannel(
            FILE_CHANNEL_ID,
            "File Transfers",
            NotificationManager.IMPORTANCE_HIGH
        )
        manager.createNotificationChannel(fileChannel)
    }
}
