package io.cdus.app.service

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import io.cdus.app.utils.Logger
import uniffi.cdus_ffi.acceptFileTransfer
import uniffi.cdus_ffi.rejectFileTransfer

class FileActionReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val fileHash = intent.getStringExtra("file_hash") ?: return
        val action = intent.action

        Logger.i("FileActionReceiver received: $action for $fileHash")

        when (action) {
            "ACCEPT" -> {
                acceptFileTransfer(fileHash)
                io.cdus.app.data.FileTransferManager.transfers[fileHash]?.let {
                    io.cdus.app.data.FileTransferManager.updateTransfer(it.copy(status = io.cdus.app.data.TransferStatus.DOWNLOADING))
                }
            }
            "DECLINE" -> {
                rejectFileTransfer(fileHash)
                io.cdus.app.data.FileTransferManager.transfers[fileHash]?.let {
                    io.cdus.app.data.FileTransferManager.updateTransfer(it.copy(status = io.cdus.app.data.TransferStatus.REJECTED))
                }
            }
        }

        // Remove notification
        val notificationManager = context.getSystemService(Context.NOTIFICATION_SERVICE) as android.app.NotificationManager
        notificationManager.cancel(2) // FILE_NOTIFICATION_ID
    }
}
