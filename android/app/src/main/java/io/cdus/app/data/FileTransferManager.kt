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
    val nodeId: String? = null,
    val senderLabel: String? = null,
    val error: String? = null,
    val speedMbps: Float? = null,
    val totalBytes: Long = 0L
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

    fun loadHistory() {
        try {
            val history = uniffi.cdus_ffi.getFileTransferHistory(50u)
            history.forEach { record ->
                val status = when (record.status) {
                    "complete" -> TransferStatus.COMPLETE
                    "failed" -> TransferStatus.ERROR
                    "declined" -> TransferStatus.REJECTED
                    "in_progress", "paused" -> {
                        // If it was in progress/paused but we just restarted, it's effectively "paused"
                        if (record.direction == "outgoing") TransferStatus.OUTGOING else TransferStatus.DOWNLOADING
                    }
                    "awaiting_acceptance" -> TransferStatus.INCOMING
                    else -> TransferStatus.ERROR
                }

                val info = FileTransferInfo(
                    transferId = record.transferId,
                    fileName = record.fileName,
                    progress = if (record.totalBytes > 0uL) (record.bytesConfirmed.toFloat() / record.totalBytes.toFloat()) * 100f else 0f,
                    status = status,
                    nodeId = record.peerNodeId,
                    error = record.errorMessage,
                    totalBytes = record.totalBytes.toLong()
                )
                transfers[record.transferId] = info
            }
        } catch (e: Exception) {
            // Log error
        }
    }
}
