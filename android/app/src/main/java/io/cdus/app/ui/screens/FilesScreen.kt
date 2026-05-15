package io.cdus.app.ui.screens

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.FileDownload
import androidx.compose.material.icons.filled.FileUpload
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Error
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import io.cdus.app.data.FileTransferManager
import io.cdus.app.data.FileTransferInfo
import io.cdus.app.data.TransferStatus

@Composable
fun FilesScreen() {
    val transfers = FileTransferManager.transfers.values.toList()

    Column(modifier = Modifier.fillMaxSize().padding(16.dp)) {
        Text(text = "File Transfers", style = MaterialTheme.typography.headlineMedium)
        Spacer(modifier = Modifier.height(16.dp))

        if (transfers.isEmpty()) {
            Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                Text(text = "No transfers yet.", color = MaterialTheme.colorScheme.outline)
            }
        } else {
            LazyColumn {
                items(transfers) { transfer ->
                    TransferItem(transfer)
                }
            }
        }
    }
}

@Composable
fun TransferItem(transfer: FileTransferInfo) {
    Card(
        modifier = Modifier.fillMaxWidth().padding(vertical = 4.dp),
        colors = CardDefaults.cardColors(
            containerColor = when(transfer.status) {
                TransferStatus.COMPLETE -> MaterialTheme.colorScheme.surfaceVariant
                TransferStatus.ERROR -> MaterialTheme.colorScheme.errorContainer
                TransferStatus.REJECTED -> MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.5f)
                else -> MaterialTheme.colorScheme.surface
            }
        )
    ) {
        Column(modifier = Modifier.padding(16.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(
                    imageVector = when(transfer.status) {
                        TransferStatus.INCOMING -> Icons.Default.FileDownload
                        TransferStatus.DOWNLOADING -> Icons.Default.FileDownload
                        TransferStatus.OUTGOING -> Icons.Default.FileUpload
                        TransferStatus.COMPLETE -> Icons.Default.CheckCircle
                        TransferStatus.ERROR -> Icons.Default.Error
                        TransferStatus.REJECTED -> Icons.Default.Error
                    },
                    contentDescription = null,
                    tint = when(transfer.status) {
                        TransferStatus.COMPLETE -> MaterialTheme.colorScheme.primary
                        TransferStatus.ERROR -> MaterialTheme.colorScheme.error
                        else -> MaterialTheme.colorScheme.secondary
                    }
                )
                Spacer(modifier = Modifier.width(16.dp))
                Column(modifier = Modifier.weight(1f)) {
                    Text(text = transfer.fileName, style = MaterialTheme.typography.bodyLarge)
                    if (transfer.status == TransferStatus.ERROR) {
                        Text(text = transfer.error ?: "Unknown error", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
                    } else if (transfer.status == TransferStatus.REJECTED) {
                        Text(text = "Declined", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.outline)
                    } else if (transfer.status != TransferStatus.INCOMING) {
                        LinearProgressIndicator(
                            progress = transfer.progress / 100f,
                            modifier = Modifier.fillMaxWidth().padding(top = 8.dp)
                        )
                        Text(
                            text = "${transfer.progress.toInt()}%",
                            style = MaterialTheme.typography.bodySmall,
                            modifier = Modifier.align(Alignment.End)
                        )
                    } else {
                        Text(text = "Waiting for your action...", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.primary)
                    }
                }
            }
            
            if (transfer.status == TransferStatus.INCOMING) {
                Spacer(modifier = Modifier.height(12.dp))
                Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                    TextButton(onClick = { 
                        uniffi.cdus_ffi.rejectFileTransfer(transfer.fileHash)
                        FileTransferManager.updateTransfer(transfer.copy(status = TransferStatus.REJECTED))
                    }) {
                        Text("Decline", color = MaterialTheme.colorScheme.error)
                    }
                    Spacer(modifier = Modifier.width(8.dp))
                    Button(onClick = { 
                        uniffi.cdus_ffi.acceptFileTransfer(transfer.fileHash)
                        FileTransferManager.updateTransfer(transfer.copy(status = TransferStatus.DOWNLOADING))
                    }) {
                        Text("Accept")
                    }
                }
            }
        }
    }
}
