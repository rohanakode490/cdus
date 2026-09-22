package io.cdus.app.ui.components

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Description
import androidx.compose.material.icons.filled.Devices
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.cdus.app.data.FileTransferInfo
import io.cdus.app.utils.UIUtils

@Composable
fun IncomingTransferDialog(
    transfer: FileTransferInfo,
    onAccept: () -> Unit,
    onDecline: () -> Unit
) {
    val sender = if (!transfer.senderLabel.isNullOrBlank()) {
        transfer.senderLabel
    } else if (!transfer.nodeId.isNullOrBlank()) {
        UIUtils.formatDeviceLabel(transfer.nodeId)
    } else {
        "Unknown Device"
    }

    val fileSizeFormatted = if (transfer.totalBytes > 0) {
        val sizeMb = transfer.totalBytes / 1024f / 1024f
        if (sizeMb < 1f) {
            val sizeKb = transfer.totalBytes / 1024f
            "%.1f KB".format(sizeKb)
        } else {
            "%.2f MB".format(sizeMb)
        }
    } else {
        "Unknown size"
    }

    AlertDialog(
        onDismissRequest = onDecline,
        shape = RoundedCornerShape(16.dp),
        containerColor = MaterialTheme.colorScheme.surface,
        icon = {
            Icon(
                imageVector = Icons.Default.Description,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.primary,
                modifier = Modifier.size(36.dp)
            )
        },
        title = {
            Text(
                text = "Incoming File Transfer",
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.Bold
            )
        },
        text = {
            Column(modifier = Modifier.fillMaxWidth()) {
                Surface(
                    color = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.5f),
                    shape = RoundedCornerShape(12.dp),
                    modifier = Modifier.fillMaxWidth()
                ) {
                    Column(modifier = Modifier.padding(14.dp)) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Icon(
                                imageVector = Icons.Default.Devices,
                                contentDescription = null,
                                modifier = Modifier.size(16.dp),
                                tint = MaterialTheme.colorScheme.outline
                            )
                            Spacer(modifier = Modifier.width(6.dp))
                            Text(
                                text = "From: $sender",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.outline
                            )
                        }
                        Spacer(modifier = Modifier.height(8.dp))
                        Text(
                            text = transfer.fileName,
                            style = MaterialTheme.typography.bodyMedium,
                            fontWeight = FontWeight.SemiBold,
                            maxLines = 2,
                            overflow = TextOverflow.Ellipsis
                        )
                        Spacer(modifier = Modifier.height(4.dp))
                        Text(
                            text = fileSizeFormatted,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.outline
                        )
                    }
                }
                Spacer(modifier = Modifier.height(10.dp))
                Text(
                    text = "End-to-end encrypted peer-to-peer transfer",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.outline.copy(alpha = 0.7f)
                )
            }
        },
        confirmButton = {
            Button(
                onClick = onAccept,
                shape = RoundedCornerShape(8.dp),
                colors = ButtonDefaults.buttonColors(
                    containerColor = MaterialTheme.colorScheme.primary,
                    contentColor = MaterialTheme.colorScheme.onPrimary
                )
            ) {
                Text("Accept", fontWeight = FontWeight.Bold)
            }
        },
        dismissButton = {
            OutlinedButton(
                onClick = onDecline,
                shape = RoundedCornerShape(8.dp),
                colors = ButtonDefaults.outlinedButtonColors(
                    contentColor = MaterialTheme.colorScheme.error
                ),
                border = BorderStroke(
                    1.dp,
                    MaterialTheme.colorScheme.error.copy(alpha = 0.5f)
                )
            ) {
                Text("Decline")
            }
        }
    )
}
