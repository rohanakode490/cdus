package io.cdus.app.ui.screens

import android.widget.Toast
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.DeleteSweep
import androidx.compose.material.icons.filled.Visibility
import androidx.compose.material.icons.filled.VisibilityOff
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.LockOpen
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.text.font.FontWeight
import java.time.LocalDateTime
import java.time.ZoneId
import java.time.ZoneOffset
import java.time.format.DateTimeFormatter
import kotlinx.coroutines.*
import com.google.accompanist.swiperefresh.SwipeRefresh
import com.google.accompanist.swiperefresh.rememberSwipeRefreshState
import uniffi.cdus_ffi.getClipboardHistory
import uniffi.cdus_ffi.deleteClipboardItem
import uniffi.cdus_ffi.clearClipboardHistory
import uniffi.cdus_ffi.ClipboardHistoryItem
import uniffi.cdus_ffi.setClipboardItemLocalOnly
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.draw.clip
import androidx.compose.foundation.shape.RoundedCornerShape

@Composable
fun ClipboardScreen() {
    var clipboardHistory by remember { mutableStateOf<List<ClipboardHistoryItem>>(emptyList()) }
    var isRefreshing by remember { mutableStateOf(false) }
    var isLoading by remember { mutableStateOf(false) }
    var errorMsg by remember { mutableStateOf<String?>(null) }
    var searchQuery by remember { mutableStateOf("") }
    val visibleSensitiveIds = remember { mutableStateListOf<Long>() }
    val scope = rememberCoroutineScope()

    fun refreshHistory() {
        scope.launch {
            isRefreshing = true
            errorMsg = null
            try {
                val freshHistory = withContext(Dispatchers.IO) {
                    getClipboardHistory(50u)
                }
                clipboardHistory = freshHistory
            } catch (e: Exception) {
                errorMsg = e.message ?: "Failed to load clipboard history"
            } finally {
                isRefreshing = false
            }
        }
    }

    LaunchedEffect(Unit) {
        isLoading = true
        errorMsg = null
        try {
            val freshHistory = withContext(Dispatchers.IO) {
                getClipboardHistory(50u)
            }
            clipboardHistory = freshHistory
        } catch (e: Exception) {
            errorMsg = e.message ?: "Failed to load clipboard history"
        } finally {
            isLoading = false
        }

        while (isActive) {
            if (!isRefreshing) {
                try {
                    val freshHistory = withContext(Dispatchers.IO) {
                        getClipboardHistory(50u)
                    }
                    clipboardHistory = freshHistory
                } catch (e: Exception) {
                    // Ignore background polling errors if we already have data
                }
            }
            delay(2000) // Poll every 2 seconds for updates
        }
    }

    val filteredHistory = remember(clipboardHistory, searchQuery) {
        if (searchQuery.isBlank()) {
            clipboardHistory
        } else {
            clipboardHistory.filter {
                it.content.contains(searchQuery, ignoreCase = true) ||
                        it.source.contains(searchQuery, ignoreCase = true)
            }
        }
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp)
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically
        ) {
            Text(
                text = "Clipboard History",
                style = MaterialTheme.typography.headlineMedium
            )
            Row(verticalAlignment = Alignment.CenterVertically) {
                IconButton(onClick = { refreshHistory() }) {
                    Icon(Icons.Default.Refresh, contentDescription = "Refresh")
                }
                if (clipboardHistory.isNotEmpty()) {
                    IconButton(onClick = {
                        scope.launch(Dispatchers.IO) {
                            clearClipboardHistory()
                            val freshHistory = getClipboardHistory(50u)
                            withContext(Dispatchers.Main) {
                                clipboardHistory = freshHistory
                            }
                        }
                    }) {
                        Icon(
                            imageVector = Icons.Default.DeleteSweep,
                            contentDescription = "Clear all history",
                            tint = MaterialTheme.colorScheme.error
                        )
                    }
                }
            }
        }

        Spacer(modifier = Modifier.height(8.dp))

        OutlinedTextField(
            value = searchQuery,
            onValueChange = { searchQuery = it },
            placeholder = { Text("Search clipboard or devices...") },
            modifier = Modifier.fillMaxWidth(),
            singleLine = true
        )

        Spacer(modifier = Modifier.height(16.dp))

        Box(modifier = Modifier.fillMaxSize()) {
            if (isLoading) {
                Column(
                    modifier = Modifier.fillMaxSize(),
                    verticalArrangement = Arrangement.Center,
                    horizontalAlignment = Alignment.CenterHorizontally
                ) {
                    CircularProgressIndicator()
                    Spacer(modifier = Modifier.height(16.dp))
                    Text(text = "Loading clipboard history...", color = MaterialTheme.colorScheme.outline)
                }
            } else if (errorMsg != null) {
                Column(
                    modifier = Modifier.fillMaxSize().padding(24.dp),
                    verticalArrangement = Arrangement.Center,
                    horizontalAlignment = Alignment.CenterHorizontally
                ) {
                    Icon(
                        imageVector = Icons.Default.Warning,
                        contentDescription = "Error",
                        tint = MaterialTheme.colorScheme.error,
                        modifier = Modifier.size(48.dp)
                    )
                    Spacer(modifier = Modifier.height(16.dp))
                    Text(
                        text = errorMsg!!,
                        color = MaterialTheme.colorScheme.error,
                        fontWeight = FontWeight.Medium,
                        fontSize = 16.sp
                    )
                    Spacer(modifier = Modifier.height(16.dp))
                    Button(onClick = { refreshHistory() }) {
                        Text("Retry")
                    }
                }
            } else {
                SwipeRefresh(
                    state = rememberSwipeRefreshState(isRefreshing),
                    onRefresh = { refreshHistory() },
                    modifier = Modifier.fillMaxSize()
                ) {
                    if (filteredHistory.isEmpty()) {
                        Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                            Column(
                                horizontalAlignment = Alignment.CenterHorizontally,
                                verticalArrangement = Arrangement.Center
                            ) {
                                Text(
                                    text = if (searchQuery.isBlank()) "No clipboard history yet." else "No matches found.",
                                    color = MaterialTheme.colorScheme.outline,
                                    fontStyle = androidx.compose.ui.text.font.FontStyle.Italic
                                )
                                Spacer(modifier = Modifier.height(16.dp))
                                Button(onClick = { refreshHistory() }) {
                                    Text("Refresh")
                                }
                            }
                        }
                    } else {
                        LazyColumn(
                            modifier = Modifier.fillMaxSize(),
                            verticalArrangement = Arrangement.spacedBy(8.dp)
                        ) {
                            items(filteredHistory, key = { it.id }) { item ->
                                val isVisible = visibleSensitiveIds.contains(item.id)
                                ClipboardListItem(
                                    item = item,
                                    isVisible = isVisible,
                                    onToggleVisibility = {
                                        if (isVisible) {
                                            visibleSensitiveIds.remove(item.id)
                                        } else {
                                            visibleSensitiveIds.add(item.id)
                                        }
                                    },
                                    onDelete = {
                                        scope.launch(Dispatchers.IO) {
                                            deleteClipboardItem(item.id)
                                            val freshHistory = getClipboardHistory(50u)
                                            withContext(Dispatchers.Main) {
                                                clipboardHistory = freshHistory
                                            }
                                        }
                                    },
                                    onToggleLocalOnly = {
                                        scope.launch(Dispatchers.IO) {
                                            setClipboardItemLocalOnly(item.id, !item.localOnly)
                                            val freshHistory = getClipboardHistory(50u)
                                            withContext(Dispatchers.Main) {
                                                clipboardHistory = freshHistory
                                            }
                                        }
                                    }
                                )
                            }
                        }
                    }
                }
            }
        }
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
fun ClipboardListItem(
    item: ClipboardHistoryItem,
    isVisible: Boolean,
    onToggleVisibility: () -> Unit,
    onDelete: () -> Unit,
    onToggleLocalOnly: () -> Unit
) {
    val clipboardManager = LocalClipboardManager.current
    val context = LocalContext.current
    
    val displayTime = remember(item.timestamp) {
        try {
            val formatter = DateTimeFormatter.ofPattern("yyyy-MM-dd HH:mm:ss")
            val utcDateTime = LocalDateTime.parse(item.timestamp, formatter)
            val zonedDateTime = utcDateTime.atZone(ZoneOffset.UTC)
            val localDateTime = zonedDateTime.withZoneSameInstant(ZoneId.systemDefault())
            localDateTime.format(DateTimeFormatter.ofPattern("MMM dd, HH:mm"))
        } catch (e: Exception) {
            item.timestamp
        }
    }

    val isSensitive = item.isSensitive

    var isImage by remember { mutableStateOf(false) }
    var isUrl by remember { mutableStateOf(false) }
    var urlText by remember { mutableStateOf("") }
    var urlTitle by remember { mutableStateOf("") }
    var faviconBitmap by remember { mutableStateOf<android.graphics.Bitmap?>(null) }
    var imageBitmap by remember { mutableStateOf<android.graphics.Bitmap?>(null) }

    val rawContentToCopy = remember(item.content) {
        if (isSensitive && !isVisible) {
            item.content
        } else {
            try {
                val json = org.json.JSONObject(item.content)
                val type = json.optString("type")
                if (type == "image") {
                    isImage = true
                    val dataUrl = json.optString("data")
                    val b64 = dataUrl.substringAfter("base64,")
                    val decodedBytes = android.util.Base64.decode(b64, android.util.Base64.DEFAULT)
                    val bmp = android.graphics.BitmapFactory.decodeByteArray(decodedBytes, 0, decodedBytes.size)
                    imageBitmap = bmp
                    item.content
                } else if (type == "url") {
                    isUrl = true
                    urlText = json.optString("url")
                    urlTitle = json.optString("title")
                    val faviconUrl = json.optString("favicon")
                    if (faviconUrl.isNotEmpty()) {
                        val b64 = faviconUrl.substringAfter("base64,")
                        val decodedBytes = android.util.Base64.decode(b64, android.util.Base64.DEFAULT)
                        val bmp = android.graphics.BitmapFactory.decodeByteArray(decodedBytes, 0, decodedBytes.size)
                        faviconBitmap = bmp
                    }
                    urlText
                } else {
                    item.content
                }
            } catch (e: Exception) {
                isImage = false
                isUrl = false
                item.content
            }
        }
    }

    Card(
        modifier = Modifier
            .fillMaxWidth()
            .combinedClickable(
                onClick = {
                    clipboardManager.setText(AnnotatedString(rawContentToCopy))
                    Toast.makeText(context, "Copied to clipboard", Toast.LENGTH_SHORT).show()
                },
                onLongClick = {
                    onToggleLocalOnly()
                    val statusText = if (!item.localOnly) "Locked to this device" else "Shared sync enabled"
                    Toast.makeText(context, statusText, Toast.LENGTH_SHORT).show()
                }
            ),
        elevation = CardDefaults.cardElevation(defaultElevation = 2.dp)
    ) {
        Column(
            modifier = Modifier.padding(16.dp)
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.Top
            ) {
                Box(modifier = Modifier.weight(1f)) {
                    if (isSensitive && !isVisible) {
                        Text(
                            text = "••••••••••••",
                            style = MaterialTheme.typography.bodyLarge
                        )
                    } else if (isImage && imageBitmap != null) {
                        androidx.compose.foundation.Image(
                            bitmap = imageBitmap!!.asImageBitmap(),
                            contentDescription = "Clipboard Image",
                            modifier = Modifier
                                .fillMaxWidth()
                                .heightIn(max = 200.dp)
                                .clip(RoundedCornerShape(8.dp)),
                            contentScale = androidx.compose.ui.layout.ContentScale.Fit
                        )
                    } else if (isUrl) {
                        Row(
                            verticalAlignment = Alignment.CenterVertically,
                            modifier = Modifier.fillMaxWidth()
                        ) {
                            if (faviconBitmap != null) {
                                androidx.compose.foundation.Image(
                                    bitmap = faviconBitmap!!.asImageBitmap(),
                                    contentDescription = null,
                                    modifier = Modifier
                                        .size(24.dp)
                                        .clip(RoundedCornerShape(4.dp))
                                )
                            } else {
                                Text("🌐", style = MaterialTheme.typography.titleMedium)
                            }
                            Spacer(modifier = Modifier.width(12.dp))
                            Column {
                                Text(
                                    text = urlTitle,
                                    style = MaterialTheme.typography.bodyLarge,
                                    fontWeight = androidx.compose.ui.text.font.FontWeight.SemiBold,
                                    maxLines = 1,
                                    overflow = TextOverflow.Ellipsis
                                )
                                Text(
                                    text = urlText,
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.primary,
                                    textDecoration = androidx.compose.ui.text.style.TextDecoration.Underline,
                                    maxLines = 1,
                                    overflow = TextOverflow.Ellipsis
                                )
                            }
                        }
                    } else {
                        Text(
                            text = item.content,
                            style = MaterialTheme.typography.bodyLarge,
                            maxLines = 2,
                            overflow = TextOverflow.Ellipsis
                        )
                    }
                }
                
                Row(verticalAlignment = Alignment.CenterVertically) {
                    IconButton(
                        onClick = onToggleLocalOnly,
                        modifier = Modifier.size(24.dp)
                    ) {
                        Icon(
                            imageVector = if (item.localOnly) Icons.Default.Lock else Icons.Default.LockOpen,
                            contentDescription = if (item.localOnly) "Shared sync disabled" else "Keep on this device only",
                            tint = if (item.localOnly) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.outline,
                            modifier = Modifier.size(18.dp)
                        )
                    }
                    Spacer(modifier = Modifier.width(8.dp))
                    if (isSensitive) {
                        IconButton(
                            onClick = onToggleVisibility,
                            modifier = Modifier.size(24.dp)
                        ) {
                            Icon(
                                imageVector = if (isVisible) Icons.Default.VisibilityOff else Icons.Default.Visibility,
                                contentDescription = if (isVisible) "Hide password" else "Show password",
                                modifier = Modifier.size(18.dp)
                              )
                        }
                        Spacer(modifier = Modifier.width(8.dp))
                    }
                    IconButton(
                        onClick = onDelete,
                        modifier = Modifier.size(24.dp)
                    ) {
                        Icon(
                            imageVector = Icons.Default.Delete,
                            contentDescription = "Delete item",
                            tint = MaterialTheme.colorScheme.error,
                            modifier = Modifier.size(18.dp)
                        )
                    }
                }
            }
            Spacer(modifier = Modifier.height(8.dp))
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween
            ) {
                Text(
                    text = "from ${item.source}",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.secondary
                )
                Text(
                    text = displayTime,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.outline
                )
            }
        }
    }
}
