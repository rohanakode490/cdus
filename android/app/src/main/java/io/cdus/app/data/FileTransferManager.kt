package io.cdus.app.data

import androidx.compose.runtime.mutableStateMapOf

enum class TransferStatus {
    INCOMING, OUTGOING, DOWNLOADING, COMPLETE, ERROR, REJECTED, HASHING
}

data class FileTransferInfo(
    val transferId: String,
    val fileName: String,
    val progress: Float,
    val status: TransferStatus,
    val error: String? = null
)

object FileTransferManager {
    val transfers = mutableStateMapOf<String, FileTransferInfo>()

    fun updateTransfer(info: FileTransferInfo) {
        transfers[info.transferId] = info
    }

    fun linkPathToId(path: String, transferId: String) {
        val current = transfers[path]
        if (current != null) {
            transfers.remove(path)
            transfers[transferId] = current.copy(transferId = transferId)
        }
    }

    fun updateProgress(transferId: String, progress: Float) {
        val current = transfers[transferId]
        if (current != null) {
            val newStatus = if (current.status == TransferStatus.INCOMING) TransferStatus.DOWNLOADING else current.status
            transfers[transferId] = current.copy(progress = progress, status = newStatus)
        }
    }

    fun markComplete(transferId: String) {
        val current = transfers[transferId]
        if (current != null) {
            transfers[transferId] = current.copy(progress = 100f, status = TransferStatus.COMPLETE)
        }
    }

    fun markError(transferId: String, error: String) {
        val current = transfers[transferId]
        if (current != null) {
            transfers[transferId] = current.copy(status = TransferStatus.ERROR, error = error)
        }
    }

    fun removeTransfer(transferId: String) {
        transfers.remove(transferId)
    }

    fun clearFinished() {
        val toRemove = transfers.filter { it.value.status == TransferStatus.COMPLETE || it.value.status == TransferStatus.ERROR || it.value.status == TransferStatus.REJECTED }.keys
        toRemove.forEach { transfers.remove(it) }
    }
}
