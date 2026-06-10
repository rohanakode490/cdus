package io.cdus.app.ui.screens

import android.widget.Toast
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.DeleteSweep
import androidx.compose.material.icons.filled.Visibility
import androidx.compose.material.icons.filled.VisibilityOff
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
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

@Composable
fun ClipboardScreen() {
    var clipboardHistory by remember { mutableStateOf<List<ClipboardHistoryItem>>(emptyList()) }
    var isRefreshing by remember { mutableStateOf(false) }
    var searchQuery by remember { mutableStateOf("") }
    val visibleSensitiveIds = remember { mutableStateListOf<Long>() }
    val scope = rememberCoroutineScope()

    fun refreshHistory() {
        scope.launch {
            isRefreshing = true
            val freshHistory = withContext(Dispatchers.IO) {
                getClipboardHistory(50u)
            }
            delay(500)
            clipboardHistory = freshHistory
            isRefreshing = false
        }
    }

    LaunchedEffect(Unit) {
        while (isActive) {
            if (!isRefreshing) {
                val freshHistory = withContext(Dispatchers.IO) {
                    getClipboardHistory(50u)
                }
                clipboardHistory = freshHistory
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

        Spacer(modifier = Modifier.height(8.dp))

        OutlinedTextField(
            value = searchQuery,
            onValueChange = { searchQuery = it },
            placeholder = { Text("Search clipboard or devices...") },
            modifier = Modifier.fillMaxWidth(),
            singleLine = true
        )

        Spacer(modifier = Modifier.height(16.dp))

        SwipeRefresh(
            state = rememberSwipeRefreshState(isRefreshing),
            onRefresh = { refreshHistory() },
            modifier = Modifier.fillMaxSize()
        ) {
            if (filteredHistory.isEmpty()) {
                Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    Text(
                        text = if (searchQuery.isBlank()) "No clipboard history yet." else "No matches found.",
                        color = MaterialTheme.colorScheme.outline
                    )
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
                            }
                        )
                    }
                }
            }
        }
    }
}

@Composable
fun ClipboardListItem(
    item: ClipboardHistoryItem,
    isVisible: Boolean,
    onToggleVisibility: () -> Unit,
    onDelete: () -> Unit
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
    val displayText = if (isSensitive && !isVisible) "••••••••••••" else item.content

    Card(
        modifier = Modifier
            .fillMaxWidth()
            .clickable {
                clipboardManager.setText(AnnotatedString(item.content))
                Toast.makeText(context, "Copied to clipboard", Toast.LENGTH_SHORT).show()
            },
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
                Text(
                    text = displayText,
                    style = MaterialTheme.typography.bodyLarge,
                    maxLines = if (isSensitive && !isVisible) 1 else 2,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f)
                )
                
                Row(verticalAlignment = Alignment.CenterVertically) {
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

