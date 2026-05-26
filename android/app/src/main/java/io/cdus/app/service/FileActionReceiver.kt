package io.cdus.app.service

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import io.cdus.app.utils.Logger
import uniffi.cdus_ffi.acceptFileTransfer
import uniffi.cdus_ffi.rejectFileTransfer

class FileActionReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val transferId = intent.getStringExtra("transfer_id") ?: return
        val action = intent.action

        Logger.i("FileActionReceiver received: $action for $transferId")

        when (action) {
            "ACCEPT" -> {
                acceptFileTransfer(transferId)
                io.cdus.app.data.FileTransferManager.transfers[transferId]?.let {
                    io.cdus.app.data.FileTransferManager.updateTransfer(it.copy(status = io.cdus.app.data.TransferStatus.DOWNLOADING))
                }
            }
            "DECLINE" -> {
                rejectFileTransfer(transferId)
                io.cdus.app.data.FileTransferManager.transfers[transferId]?.let {
                    io.cdus.app.data.FileTransferManager.updateTransfer(it.copy(status = io.cdus.app.data.TransferStatus.REJECTED))
                }
            }
        }

        // Remove notification
        val notificationManager = context.getSystemService(Context.NOTIFICATION_SERVICE) as android.app.NotificationManager
        notificationManager.cancel(2) // FILE_NOTIFICATION_ID
    }
}
