package io.cdus.app.ui.screens

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.border
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.FileDownload
import androidx.compose.material.icons.filled.FileUpload
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Error
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material.icons.filled.Folder
import androidx.compose.material.icons.filled.Sort
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.platform.LocalContext
import android.app.NotificationManager
import android.content.Context
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp
import io.cdus.app.data.FileTransferManager
import io.cdus.app.data.FileTransferInfo
import io.cdus.app.data.TransferStatus
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import io.cdus.app.utils.UIUtils
import io.cdus.app.utils.Logger
import io.cdus.app.utils.FileUtils

enum class SortOption(val label: String) {
    NEWEST("Newest First"),
    OLDEST("Oldest First"),
    NAME_ASC("Name (A-Z)"),
    NAME_DESC("Name (Z-A)"),
    SIZE_DESC("Size (Largest First)"),
    SIZE_ASC("Size (Smallest First)")
}

@Composable
fun FilesScreen() {
    var currentSortOption by remember { mutableStateOf(SortOption.NEWEST) }
    var sortMenuExpanded by remember { mutableStateOf(false) }

    var isLoading by remember { mutableStateOf(false) }
    var errorMsg by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()

    fun refreshHistory() {
        scope.launch {
            isLoading = true
            errorMsg = null
            try {
                withContext(Dispatchers.IO) {
                    FileTransferManager.loadHistory()
                }
            } catch (e: Exception) {
                errorMsg = e.message ?: "Failed to load file transfers"
            } finally {
                isLoading = false
            }
        }
    }

    LaunchedEffect(Unit) {
        refreshHistory()
    }

    val rawTransfers = FileTransferManager.transfers.values.toList()
    val sortedTransfers = remember(rawTransfers, currentSortOption) {
        when (currentSortOption) {
            SortOption.NEWEST -> rawTransfers.reversed()
            SortOption.OLDEST -> rawTransfers
            SortOption.NAME_ASC -> rawTransfers.sortedWith(compareBy(String.CASE_INSENSITIVE_ORDER) { it.fileName })
            SortOption.NAME_DESC -> rawTransfers.sortedWith(compareByDescending(String.CASE_INSENSITIVE_ORDER) { it.fileName })
            SortOption.SIZE_DESC -> rawTransfers.sortedByDescending { it.totalBytes }
            SortOption.SIZE_ASC -> rawTransfers.sortedBy { it.totalBytes }
        }
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp)
    ) {
        // Header
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(bottom = 12.dp),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically
        ) {
            Column {
                Text(
                    text = "File Transfers",
                    style = MaterialTheme.typography.headlineMedium,
                    color = MaterialTheme.colorScheme.onBackground
                )
                Text(
                    text = "Chunked BLAKE3 P2P Stream",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
            
            Row(verticalAlignment = Alignment.CenterVertically) {
                IconButton(onClick = { refreshHistory() }) {
                    Icon(Icons.Default.Refresh, contentDescription = "Refresh", tint = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                Box {
                    OutlinedButton(
                        onClick = { sortMenuExpanded = true },
                        shape = RoundedCornerShape(8.dp),
                        contentPadding = PaddingValues(horizontal = 10.dp, vertical = 4.dp),
                        modifier = Modifier.height(34.dp)
                    ) {
                        Icon(Icons.Default.Sort, contentDescription = null, modifier = Modifier.size(14.dp))
                        Spacer(modifier = Modifier.width(4.dp))
                        Text(currentSortOption.label, style = MaterialTheme.typography.labelSmall)
                    }
                    DropdownMenu(
                        expanded = sortMenuExpanded,
                        onDismissRequest = { sortMenuExpanded = false }
                    ) {
                        SortOption.values().forEach { option ->
                            DropdownMenuItem(
                                text = { Text(option.label) },
                                onClick = {
                                    currentSortOption = option
                                    sortMenuExpanded = false
                                }
                            )
                        }
                    }
                }
                
                if (rawTransfers.any { it.status == TransferStatus.COMPLETE || it.status == TransferStatus.ERROR || it.status == TransferStatus.REJECTED }) {
                    Spacer(modifier = Modifier.width(6.dp))
                    TextButton(
                        onClick = { FileTransferManager.clearFinished() },
                        contentPadding = PaddingValues(horizontal = 8.dp, vertical = 4.dp),
                        modifier = Modifier.height(34.dp)
                    ) {
                        Text("Clear", style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.primary)
                    }
                }
            }
        }

        Box(modifier = Modifier.fillMaxSize()) {
            if (isLoading) {
                Column(
                    modifier = Modifier.fillMaxSize(),
                    verticalArrangement = Arrangement.Center,
                    horizontalAlignment = Alignment.CenterHorizontally
                ) {
                    CircularProgressIndicator(modifier = Modifier.size(28.dp), color = MaterialTheme.colorScheme.primary)
                    Spacer(modifier = Modifier.height(12.dp))
                    Text(
                        text = "Loading transfers...",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            } else if (errorMsg != null) {
                Column(
                    modifier = Modifier
                        .fillMaxSize()
                        .padding(24.dp),
                    verticalArrangement = Arrangement.Center,
                    horizontalAlignment = Alignment.CenterHorizontally
                ) {
                    Icon(
                        imageVector = Icons.Default.Warning,
                        contentDescription = "Error",
                        tint = MaterialTheme.colorScheme.error,
                        modifier = Modifier.size(44.dp)
                    )
                    Spacer(modifier = Modifier.height(12.dp))
                    Text(
                        text = errorMsg!!,
                        color = MaterialTheme.colorScheme.error,
                        fontWeight = FontWeight.Medium,
                        style = MaterialTheme.typography.bodyMedium
                    )
                    Spacer(modifier = Modifier.height(16.dp))
                    Button(onClick = { refreshHistory() }) {
                        Text("Retry")
                    }
                }
            } else if (sortedTransfers.isEmpty()) {
                Box(
                    modifier = Modifier.fillMaxSize(),
                    contentAlignment = Alignment.Center
                ) {
                    Card(
                        modifier = Modifier
                            .fillMaxWidth()
                            .border(1.dp, MaterialTheme.colorScheme.outline, RoundedCornerShape(14.dp)),
                        shape = RoundedCornerShape(14.dp),
                        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface)
                    ) {
                        Column(
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(28.dp),
                            horizontalAlignment = Alignment.CenterHorizontally
                        ) {
                            Surface(
                                shape = RoundedCornerShape(12.dp),
                                color = MaterialTheme.colorScheme.surfaceVariant,
                                modifier = Modifier.size(54.dp)
                            ) {
                                Box(contentAlignment = Alignment.Center) {
                                    Icon(
                                        imageVector = Icons.Default.Folder,
                                        contentDescription = null,
                                        modifier = Modifier.size(26.dp),
                                        tint = MaterialTheme.colorScheme.primary
                                    )
                                }
                            }
                            Spacer(modifier = Modifier.height(12.dp))
                            Text(
                                text = "No file transfers yet",
                                style = MaterialTheme.typography.titleMedium,
                                color = MaterialTheme.colorScheme.onSurface
                            )
                            Spacer(modifier = Modifier.height(6.dp))
                            Text(
                                text = "Open the Devices tab, select any paired laptop or phone, and tap Send File to transfer directly without cloud storage.",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                textAlign = androidx.compose.ui.text.style.TextAlign.Center
                            )
                        }
                    }
                }
            } else {
                LazyColumn(
                    modifier = Modifier.fillMaxSize(),
                    verticalArrangement = Arrangement.spacedBy(10.dp)
                ) {
                    items(sortedTransfers) { transfer ->
                        TransferItem(transfer)
                    }
                }
            }
        }
    }
}

@Composable
fun TransferItem(transfer: FileTransferInfo) {
    val context = LocalContext.current
    var showMenu by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    val (iconBgColor, iconTintColor) = when (transfer.status) {
        TransferStatus.COMPLETE -> MaterialTheme.colorScheme.secondaryContainer to MaterialTheme.colorScheme.secondary
        TransferStatus.ERROR, TransferStatus.REJECTED -> MaterialTheme.colorScheme.errorContainer to MaterialTheme.colorScheme.error
        else -> MaterialTheme.colorScheme.primaryContainer to MaterialTheme.colorScheme.primary
    }

    Card(
        modifier = Modifier
            .fillMaxWidth()
            .border(1.dp, MaterialTheme.colorScheme.outline, RoundedCornerShape(12.dp)),
        shape = RoundedCornerShape(12.dp),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surface
        )
    ) {
        Column(modifier = Modifier.padding(14.dp)) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                modifier = Modifier.fillMaxWidth()
            ) {
                Surface(
                    shape = RoundedCornerShape(10.dp),
                    color = iconBgColor,
                    modifier = Modifier.size(40.dp)
                ) {
                    Box(contentAlignment = Alignment.Center) {
                        Icon(
                            imageVector = when(transfer.status) {
                                TransferStatus.INCOMING -> Icons.Default.FileDownload
                                TransferStatus.DOWNLOADING -> Icons.Default.FileDownload
                                TransferStatus.OUTGOING -> Icons.Default.FileUpload
                                TransferStatus.HASHING -> Icons.Default.FileUpload
                                TransferStatus.COMPLETE -> Icons.Default.CheckCircle
                                TransferStatus.ERROR -> Icons.Default.Error
                                TransferStatus.REJECTED -> Icons.Default.Error
                            },
                            contentDescription = null,
                            tint = iconTintColor,
                            modifier = Modifier.size(20.dp)
                        )
                    }
                }
                Spacer(modifier = Modifier.width(12.dp))
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        text = transfer.fileName,
                        style = MaterialTheme.typography.titleSmall,
                        fontWeight = FontWeight.SemiBold,
                        color = MaterialTheme.colorScheme.onSurface,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis
                    )
                    if (transfer.status == TransferStatus.ERROR) {
                        Text(
                            text = UIUtils.sanitizeErrorMessage(transfer.error),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.error,
                            maxLines = 2,
                            overflow = TextOverflow.Ellipsis
                        )
                    } else if (transfer.status == TransferStatus.REJECTED) {
                        Text(
                            text = "Transfer declined",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    } else if (transfer.status == TransferStatus.HASHING) {
                        LinearProgressIndicator(
                            progress = transfer.progress / 100f,
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(top = 6.dp),
                            color = MaterialTheme.colorScheme.secondary
                        )
                        Text(
                            text = "Hashing chunks... ${transfer.progress.toInt()}%",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.align(Alignment.End)
                        )
                    } else if (transfer.status != TransferStatus.INCOMING) {
                        LinearProgressIndicator(
                            progress = transfer.progress / 100f,
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(top = 6.dp)
                        )
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(top = 4.dp),
                            horizontalArrangement = Arrangement.SpaceBetween,
                            verticalAlignment = Alignment.CenterVertically
                        ) {
                            Text(
                                text = if (transfer.speedMbps != null) "%.1f Mbps".format(transfer.speedMbps) else "",
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.tertiary
                            )
                            Text(
                                text = "${transfer.progress.toInt()}%",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                        }
                    } else {
                        Text(
                            text = "Incoming file from peer...",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.primary
                        )
                    }
                }
                
                if (transfer.status != TransferStatus.INCOMING) {
                    Box {
                        IconButton(onClick = { showMenu = true }, modifier = Modifier.size(36.dp)) {
                            Icon(
                                imageVector = Icons.Default.MoreVert,
                                contentDescription = "Options",
                                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                                modifier = Modifier.size(18.dp)
                            )
                        }

                        DropdownMenu(
                            expanded = showMenu,
                            onDismissRequest = { showMenu = false }
                        ) {
                            when (transfer.status) {
                                TransferStatus.OUTGOING, TransferStatus.DOWNLOADING, TransferStatus.HASHING -> {
                                    DropdownMenuItem(
                                        text = { Text("Cancel", color = MaterialTheme.colorScheme.error) },
                                        onClick = {
                                            showMenu = false
                                            uniffi.cdus_ffi.cancelFileTransfer(transfer.transferId)
                                            FileTransferManager.markError(transfer.transferId, "Cancelled")
                                        }
                                    )
                                }
                                TransferStatus.COMPLETE, TransferStatus.ERROR, TransferStatus.REJECTED -> {
                                    if (transfer.status == TransferStatus.COMPLETE) {
                                        DropdownMenuItem(
                                            text = { Text("Open File") },
                                            onClick = {
                                                showMenu = false
                                                FileUtils.openFile(context, transfer.fileName)
                                            }
                                        )
                                        DropdownMenuItem(
                                            text = { Text("Show in Folder") },
                                            onClick = {
                                                showMenu = false
                                                FileUtils.openFileLocation(context, transfer.fileName)
                                            }
                                        )
                                    }
                                    DropdownMenuItem(
                                        text = { Text("Dismiss") },
                                        onClick = {
                                            showMenu = false
                                            FileTransferManager.removeTransfer(transfer.transferId)
                                        }
                                    )
                                    DropdownMenuItem(
                                        text = { Text("Delete Permanently", color = MaterialTheme.colorScheme.error) },
                                        onClick = {
                                            showMenu = false
                                            FileTransferManager.deleteTransferPermanently(context, transfer.transferId)
                                        }
                                    )
                                }
                                else -> {}
                            }
                        }
                    }
                }
            }
            
            if (transfer.status == TransferStatus.INCOMING) {
                Spacer(modifier = Modifier.height(12.dp))
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.End
                ) {
                    TextButton(
                        onClick = {
                            uniffi.cdus_ffi.acceptFileTransfer(transfer.transferId)
                            FileTransferManager.updateTransfer(transfer.copy(status = TransferStatus.DOWNLOADING))
                            val notificationManager = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
                            notificationManager.cancel(2) // FILE_NOTIFICATION_ID
                        }
                    ) {
                        Text("Accept")
                    }
                    Spacer(modifier = Modifier.width(8.dp))
                    TextButton(
                        onClick = {
                            uniffi.cdus_ffi.rejectFileTransfer(transfer.transferId)
                            FileTransferManager.updateTransfer(transfer.copy(status = TransferStatus.REJECTED))
                            val notificationManager = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
                            notificationManager.cancel(2) // FILE_NOTIFICATION_ID
                        }
                    ) {
                        Text("Decline", color = MaterialTheme.colorScheme.error)
                    }
                }
            }

            if (transfer.status == TransferStatus.ERROR) {
                Spacer(modifier = Modifier.height(12.dp))
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.End,
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    TextButton(
                        onClick = {
                            val intent = android.content.Intent(android.content.Intent.ACTION_VIEW, android.net.Uri.parse("https://github.com/rohanakode490/cdus/blob/main/docs/troubleshooting.md"))
                            context.startActivity(intent)
                        }
                    ) {
                        Text("Troubleshoot", color = MaterialTheme.colorScheme.outline)
                    }
                    Spacer(modifier = Modifier.width(8.dp))
                    Button(
                        onClick = {
                            scope.launch {
                                withContext(Dispatchers.IO) {
                                    try {
                                        uniffi.cdus_ffi.resumeFileTransfer(transfer.transferId)
                                    } catch (e: Exception) {
                                        Logger.e("Failed to resume transfer: ${e.message}")
                                    }
                                }
                                FileTransferManager.updateTransfer(transfer.copy(status = TransferStatus.DOWNLOADING, progress = 0f, error = null))
                            }
                        }
                    ) {
                        Text("Retry")
                    }
                }
            }
        }
    }
}
