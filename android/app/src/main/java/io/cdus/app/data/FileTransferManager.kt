package io.cdus.app.data

import androidx.compose.runtime.mutableStateMapOf

enum class TransferStatus {
    INCOMING, OUTGOING, DOWNLOADING, COMPLETE, ERROR, REJECTED
}

data class FileTransferInfo(
    val fileHash: String,
    val fileName: String,
    val progress: Float,
    val status: TransferStatus,
    val error: String? = null
)

object FileTransferManager {
    val transfers = mutableStateMapOf<String, FileTransferInfo>()

    fun updateTransfer(info: FileTransferInfo) {
        transfers[info.fileHash] = info
    }

    fun updateProgress(fileHash: String, progress: Float) {
        val current = transfers[fileHash]
        if (current != null) {
            val newStatus = if (current.status == TransferStatus.INCOMING) TransferStatus.DOWNLOADING else current.status
            transfers[fileHash] = current.copy(progress = progress, status = newStatus)
        }
    }

    fun markComplete(fileHash: String) {
        val current = transfers[fileHash]
        if (current != null) {
            transfers[fileHash] = current.copy(progress = 100f, status = TransferStatus.COMPLETE)
        }
    }

    fun markError(fileHash: String, error: String) {
        val current = transfers[fileHash]
        if (current != null) {
            transfers[fileHash] = current.copy(status = TransferStatus.ERROR, error = error)
        }
    }
}
