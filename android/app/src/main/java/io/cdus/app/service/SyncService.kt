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
import uniffi.cdus_ffi.setClipboardListener
import uniffi.cdus_ffi.ClipboardListener
import uniffi.cdus_ffi.FileTransferListener
import uniffi.cdus_ffi.setFileTransferListener
import uniffi.cdus_ffi.acceptFileTransfer
import uniffi.cdus_ffi.rejectFileTransfer
import io.cdus.app.data.FileTransferManager
import io.cdus.app.data.FileTransferInfo
import io.cdus.app.data.TransferStatus
import io.cdus.app.utils.FileUtils
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

    override fun onIncomingRequest(nodeId: String, transferId: String, fileName: String, totalBytes: ULong, senderLabel: String) {
        Logger.i("Incoming file request from $nodeId: $fileName")
        
        FileTransferManager.updateTransfer(
            FileTransferInfo(
                transferId = transferId,
                fileName = fileName,
                progress = 0f,
                status = TransferStatus.INCOMING
            )
        )

        showIncomingNotification(nodeId, transferId, fileName, totalBytes.toLong())
    }

    private fun showIncomingNotification(nodeId: String, transferId: String, fileName: String, totalSize: Long) {
        // Show notification with Accept/Decline actions
        val acceptIntent = Intent(this, FileActionReceiver::class.java).apply {
            action = "ACCEPT"
            putExtra("transfer_id", transferId)
        }
        val acceptPendingIntent = android.app.PendingIntent.getBroadcast(
            this, 
            transferId.hashCode(), 
            acceptIntent, 
            android.app.PendingIntent.FLAG_IMMUTABLE or android.app.PendingIntent.FLAG_UPDATE_CURRENT
        )

        val declineIntent = Intent(this, FileActionReceiver::class.java).apply {
            action = "DECLINE"
            putExtra("transfer_id", transferId)
        }
        val declinePendingIntent = android.app.PendingIntent.getBroadcast(
            this, 
            transferId.hashCode() + 1, 
            declineIntent, 
            android.app.PendingIntent.FLAG_IMMUTABLE or android.app.PendingIntent.FLAG_UPDATE_CURRENT
        )

        val sizeMb = totalSize / 1024f / 1024f
        val notification = NotificationCompat.Builder(this, FILE_CHANNEL_ID)
            .setSmallIcon(android.R.drawable.stat_sys_download)
            .setContentTitle("Incoming File")
            .setContentText("$fileName (%.2f MB) from ${io.cdus.app.data.DeviceManager.getLabel(nodeId)}".format(sizeMb))
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .addAction(android.R.drawable.checkbox_on_background, "Accept", acceptPendingIntent)
            .addAction(android.R.drawable.ic_menu_close_clear_cancel, "Decline", declinePendingIntent)
            .setAutoCancel(true)
            .build()

        val notificationManager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        notificationManager.notify(FILE_NOTIFICATION_ID, notification)
    }

    override fun onIncomingTransferStarted(transferId: String, fileName: String, totalBytes: ULong) {
        Logger.i("Incoming file transfer started: $fileName")
        
        FileTransferManager.updateTransfer(
            FileTransferInfo(
                transferId = transferId,
                fileName = fileName,
                progress = 0f,
                status = TransferStatus.DOWNLOADING
            )
        )
    }

    override fun onOutgoingTransferStarted(transferId: String, fileName: String, totalBytes: ULong) {
        Logger.i("Outgoing file transfer started: $fileName")
        
        // Find existing entry by fileName that is in HASHING status
        val existing = FileTransferManager.transfers.values.find { 
            it.fileName == fileName && it.status == TransferStatus.HASHING 
        }
        
        if (existing != null) {
            FileTransferManager.linkPathToId(existing.transferId, transferId)
            FileTransferManager.updateProgress(transferId, 0f) // Reset to 0% for actual transfer
        } else {
            FileTransferManager.updateTransfer(
                FileTransferInfo(
                    transferId = transferId,
                    fileName = fileName,
                    progress = 0f,
                    status = TransferStatus.OUTGOING
                )
            )
        }
    }

    override fun onTransferProgress(transferId: String, progress: Float) {
        Logger.d("Transfer progress for $transferId: $progress%")
        FileTransferManager.updateProgress(transferId, progress)

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

    override fun onTransferComplete(transferId: String, destPath: String) {
        Logger.i("Transfer complete: $transferId to $destPath")
        FileTransferManager.markComplete(transferId)
        
        val info = FileTransferManager.transfers[transferId]
        if (info != null) {
            val internalFile = java.io.File(destPath)
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

    override fun onTransferError(transferId: String, error: String) {
        Logger.e("Transfer error for $transferId: $error")
        FileTransferManager.markError(transferId, error)

        val notification = NotificationCompat.Builder(this, FILE_CHANNEL_ID)
            .setSmallIcon(android.R.drawable.ic_dialog_alert)
            .setContentTitle("Transfer Failed")
            .setContentText(error)
            .setPriority(NotificationCompat.PRIORITY_DEFAULT)
            .build()

        val notificationManager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        notificationManager.notify(FILE_NOTIFICATION_ID, notification)
    }

    override fun onPeerAccepted(nodeId: String, transferId: String) {
        val label = io.cdus.app.data.DeviceManager.getLabel(nodeId)
        Logger.i("Peer $label ($nodeId) accepted file $transferId")
        
        FileTransferManager.transfers[transferId]?.let {
            FileTransferManager.updateTransfer(it.copy(status = TransferStatus.OUTGOING))
        }

        val notification = NotificationCompat.Builder(this, FILE_CHANNEL_ID)
            .setSmallIcon(android.R.drawable.stat_sys_upload)
            .setContentTitle("File Accepted")
            .setContentText("$label accepted your file. Starting transfer...")
            .setPriority(NotificationCompat.PRIORITY_DEFAULT)
            .setAutoCancel(true)
            .build()

        val notificationManager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        notificationManager.notify(FILE_NOTIFICATION_ID, notification)
    }

    override fun onPeerRejected(nodeId: String, transferId: String) {
        val label = io.cdus.app.data.DeviceManager.getLabel(nodeId)
        Logger.w("Peer $label ($nodeId) rejected file $transferId")
        
        FileTransferManager.markError(transferId, "Rejected by $label")

        val notification = NotificationCompat.Builder(this, FILE_CHANNEL_ID)
            .setSmallIcon(android.R.drawable.ic_dialog_alert)
            .setContentTitle("Transfer Rejected")
            .setContentText("$label declined the file.")
            .setPriority(NotificationCompat.PRIORITY_DEFAULT)
            .setAutoCancel(true)
            .build()

        val notificationManager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        notificationManager.notify(FILE_NOTIFICATION_ID, notification)
    }

    override fun onPeerDisconnected(nodeId: String) {
        val label = io.cdus.app.data.DeviceManager.getLabel(nodeId)
        Logger.i("Peer disconnected: $label ($nodeId)")
        android.os.Handler(android.os.Looper.getMainLooper()).post {
            android.widget.Toast.makeText(this, "$label went offline", android.widget.Toast.LENGTH_SHORT).show()
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
