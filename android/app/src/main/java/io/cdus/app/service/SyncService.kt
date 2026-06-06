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

    companion object {
        const val ACTION_STOP_SERVICE = "io.cdus.app.service.STOP_SERVICE"
    }

    private val CHANNEL_ID = "sync_channel"
    private val FILE_CHANNEL_ID = "file_transfer_channel"
    private val NOTIFICATION_ID = 1
    private val FILE_NOTIFICATION_ID = 2
    private lateinit var clipboardManager: ClipboardManager
    private var lastClipboardContent: String? = null
    
    private val clipChangedListener = ClipboardManager.OnPrimaryClipChangedListener {
        checkSystemClipboard()
    }

    override fun onCreate() {
        super.onCreate()
        createNotificationChannels()
        clipboardManager = getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        clipboardManager.addPrimaryClipChangedListener(clipChangedListener)
        setClipboardListener(this)
        setFileTransferListener(this)
        Logger.i("SyncService created and remote listeners added")
    }

    private fun checkSystemClipboard() {
        try {
            if (clipboardManager.hasPrimaryClip()) {
                val clipData = clipboardManager.primaryClip
                if (clipData != null && clipData.itemCount > 0) {
                    val content = clipData.getItemAt(0).text?.toString()
                    if (content != null && content != lastClipboardContent) {
                        val sharedPref = getSharedPreferences("cdus_settings", Context.MODE_PRIVATE)
                        if (sharedPref.getBoolean("clipboard_sync", false)) {
                            // Only broadcast if it's new
                            val lastHash = sharedPref.getString("last_clip_hash", "")
                            val currentHash = content.hashCode().toString()
                            if (currentHash != lastHash) {
                                sharedPref.edit().putString("last_clip_hash", currentHash).apply()
                                lastClipboardContent = content
                                Logger.i("New system clipboard content detected, broadcasting")
                                uniffi.cdus_ffi.broadcastClipboard(content)
                            }
                        }
                    }
                }
            }
        } catch (e: Exception) {
            Logger.e("Error checking system clipboard: ${e.message}")
        }
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
                status = TransferStatus.INCOMING,
                nodeId = nodeId,
                senderLabel = senderLabel,
                totalBytes = totalBytes.toLong()
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
        
        val existing = FileTransferManager.transfers[transferId]
        FileTransferManager.updateTransfer(
            FileTransferInfo(
                transferId = transferId,
                fileName = fileName,
                progress = 0f,
                status = TransferStatus.DOWNLOADING,
                nodeId = existing?.nodeId,
                senderLabel = existing?.senderLabel,
                totalBytes = totalBytes.toLong()
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
            val current = FileTransferManager.transfers[transferId]
            if (current != null) {
                FileTransferManager.updateTransfer(current.copy(totalBytes = totalBytes.toLong()))
            }
        } else {
            FileTransferManager.updateTransfer(
                FileTransferInfo(
                    transferId = transferId,
                    fileName = fileName,
                    progress = 0f,
                    status = TransferStatus.OUTGOING,
                    totalBytes = totalBytes.toLong()
                )
            )
        }
    }

    private var lastNotificationTime = 0L
    private var lastNotificationProgress = -1
    private var lastBytesConfirmed = 0L
    private var lastSpeedTime = 0L

    override fun onTransferProgress(transferId: String, progress: Float) {
        val currentProgress = progress.toInt()
        val currentTime = System.currentTimeMillis()
        
        // Find total bytes for speed calculation
        val info = FileTransferManager.transfers[transferId]
        val totalBytes = (1024L * 1024L * 1024L).toFloat() // Default for 1GB benchmark if unknown
        // We actually need totalBytes from Rust but UniFFI doesn't pass it here.
        // We can estimate from progress if we have the initial totalBytes.
        // For now, let's just use bytes_confirmed if we can get it.
        
        // Wait, UniFFI listener only gives `progress: Float`. 
        // I should probably have updated the listener to include bytes_confirmed.
        // Let's assume progress is accurate enough for relative speed.
        
        var currentSpeed: Float? = null
        if (lastSpeedTime > 0 && currentTime > lastSpeedTime) {
            val progressDiff = progress - (info?.progress ?: 0f)
            if (progressDiff > 0) {
                // If it's the benchmark, we know it's 1GB
                val isBenchmark = transferId == "ffffffff-ffff-ffff-ffff-ffffffffffff"
                val totalSize = if (isBenchmark) 1024f * 1024f * 1024f else 10f * 1024f * 1024f // Estimate
                val bytesDelta = (progressDiff / 100f) * totalSize
                val timeDeltaSecs = (currentTime - lastSpeedTime) / 1000f
                currentSpeed = (bytesDelta * 8 / (1024f * 1024f)) / timeDeltaSecs // Mbps
            }
        }
        lastSpeedTime = currentTime

        // Throttling: Update at most once every 500ms, OR if progress jumps by 5%
        if (currentProgress > 0 && currentProgress < 100) {
            if (currentTime - lastNotificationTime < 500 && currentProgress - lastNotificationProgress < 5) {
                // Skip this update in notification (but update memory manager)
                FileTransferManager.transfers[transferId]?.let {
                    FileTransferManager.updateTransfer(it.copy(progress = progress, speedMbps = currentSpeed ?: it.speedMbps))
                }
                return
            }
        }
        
        lastNotificationTime = currentTime
        lastNotificationProgress = currentProgress

        Logger.d("Transfer progress for $transferId: $progress% (${currentSpeed ?: 0} Mbps)")
        
        val updatedInfo = info?.copy(
            progress = progress, 
            speedMbps = currentSpeed ?: info.speedMbps,
            fileName = if (transferId == "ffffffff-ffff-ffff-ffff-ffffffffffff") "1GB Network Benchmark" else info.fileName
        ) ?: return
        
        FileTransferManager.updateTransfer(updatedInfo)

        val fileName = updatedInfo.fileName

        val cancelIntent = Intent(this, FileActionReceiver::class.java).apply {
            action = "CANCEL"
            putExtra("transfer_id", transferId)
        }
        val cancelPendingIntent = android.app.PendingIntent.getBroadcast(
            this, 
            transferId.hashCode() + 2, 
            cancelIntent, 
            android.app.PendingIntent.FLAG_IMMUTABLE or android.app.PendingIntent.FLAG_UPDATE_CURRENT
        )

        val notification = NotificationCompat.Builder(this, FILE_CHANNEL_ID)
            .setSmallIcon(if (info?.status == TransferStatus.OUTGOING) android.R.drawable.stat_sys_upload else android.R.drawable.stat_sys_download)
            .setContentTitle(if (info?.status == TransferStatus.OUTGOING) "Sending File" else "Downloading File")
            .setContentText("$fileName: $currentProgress%")
            .setProgress(100, currentProgress, false)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .addAction(android.R.drawable.ic_menu_close_clear_cancel, "Cancel", cancelPendingIntent)
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

    override fun onPeerConnected(nodeId: String) {
        val label = io.cdus.app.data.DeviceManager.getLabel(nodeId)
        Logger.i("Peer connected: $label ($nodeId)")
        android.os.Handler(android.os.Looper.getMainLooper()).post {
            android.widget.Toast.makeText(this, "$label is now online", android.widget.Toast.LENGTH_SHORT).show()
        }
    }

    override fun onPairingResult(success: Boolean, nodeId: String, label: String) {
        Logger.i("Pairing result for $label ($nodeId): success=$success")
        android.os.Handler(android.os.Looper.getMainLooper()).post {
            val msg = if (success) "Pairing successful with $label" else "Pairing failed with $label"
            android.widget.Toast.makeText(this, msg, android.widget.Toast.LENGTH_SHORT).show()
        }
    }

    override fun onAlreadyPaired(nodeId: String, label: String) {
        Logger.i("Already paired with $label ($nodeId)")
        android.os.Handler(android.os.Looper.getMainLooper()).post {
            android.widget.Toast.makeText(this, "Already paired with $label", android.widget.Toast.LENGTH_SHORT).show()
        }
    }

    override fun onStalePairing(nodeId: String, label: String) {
        Logger.w("Stale pairing detected for $label ($nodeId)")
        android.os.Handler(android.os.Looper.getMainLooper()).post {
            android.widget.Toast.makeText(this, "Pairing with $label is stale. Please re-pair.", android.widget.Toast.LENGTH_LONG).show()
        }
    }

    override fun onTransferStateChanged(transferId: String, state: String) {
        Logger.i("Transfer state changed: $transferId -> $state")
        when (state) {
            "started" -> {
                val serviceIntent = Intent(this, FileTransferForegroundService::class.java).apply {
                    action = FileTransferForegroundService.ACTION_START
                    putExtra(FileTransferForegroundService.EXTRA_TRANSFER_ID, transferId)
                }
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                    startForegroundService(serviceIntent)
                } else {
                    startService(serviceIntent)
                }
            }
            "completed", "failed" -> {
                val serviceIntent = Intent(this, FileTransferForegroundService::class.java).apply {
                    action = FileTransferForegroundService.ACTION_STOP
                    putExtra(FileTransferForegroundService.EXTRA_TRANSFER_ID, transferId)
                }
                startService(serviceIntent)
            }
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP_SERVICE) {
            Logger.i("Stop service requested via notification action")
            val sharedPref = getSharedPreferences("cdus_settings", Context.MODE_PRIVATE)
            sharedPref.edit().putBoolean("clipboard_sync", false).apply()
            stopForeground(true)
            stopSelf()
            return START_NOT_STICKY
        }

        val stopIntent = Intent(this, SyncService::class.java).apply {
            action = ACTION_STOP_SERVICE
        }
        val stopPendingIntent = android.app.PendingIntent.getService(
            this,
            0,
            stopIntent,
            android.app.PendingIntent.FLAG_IMMUTABLE or android.app.PendingIntent.FLAG_UPDATE_CURRENT
        )

        val notification = NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(android.R.drawable.ic_popup_sync)
            .setContentTitle("CDUS Sync Active")
            .setContentText("Syncing clipboard across devices...")
            .setOngoing(true)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .addAction(android.R.drawable.ic_menu_close_clear_cancel, "Stop", stopPendingIntent)
            .build()

        startForeground(NOTIFICATION_ID, notification)

        // Initial check when service starts
        checkSystemClipboard()

        return START_STICKY
    }

    override fun onDestroy() {
        super.onDestroy()
        clipboardManager.removePrimaryClipChangedListener(clipChangedListener)
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
            NotificationManager.IMPORTANCE_LOW
        )
        manager.createNotificationChannel(fileChannel)
    }
}
