package io.cdus.app.ui.screens

import android.widget.Toast
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
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
import uniffi.cdus_ffi.ClipboardHistoryItem

@Composable
fun ClipboardScreen() {
    var clipboardHistory by remember { mutableStateOf<List<ClipboardHistoryItem>>(emptyList()) }
    var isRefreshing by remember { mutableStateOf(false) }
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
                clipboardHistory = getClipboardHistory(50u)
            }
            delay(2000) // Poll every 2 seconds for updates
        }
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp)
    ) {
        Text(
            text = "Clipboard History",
            style = MaterialTheme.typography.headlineMedium,
            modifier = Modifier.padding(bottom = 16.dp)
        )

        SwipeRefresh(
            state = rememberSwipeRefreshState(isRefreshing),
            onRefresh = { refreshHistory() },
            modifier = Modifier.fillMaxSize()
        ) {
            if (clipboardHistory.isEmpty()) {
                Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    Text(text = "No clipboard history yet.", color = MaterialTheme.colorScheme.outline)
                }
            } else {
                LazyColumn(
                    modifier = Modifier.fillMaxSize(),
                    verticalArrangement = Arrangement.spacedBy(8.dp)
                ) {
                    items(clipboardHistory) { item ->
                        ClipboardListItem(item)
                    }
                }
            }
        }
    }
}

@Composable
fun ClipboardListItem(item: ClipboardHistoryItem) {
    val clipboardManager = LocalClipboardManager.current
    val context = LocalContext.current
    
    val displayTime = remember(item.timestamp) {
        try {
            // SQLite CURRENT_TIMESTAMP is "yyyy-MM-dd HH:mm:ss" in UTC
            val formatter = DateTimeFormatter.ofPattern("yyyy-MM-dd HH:mm:ss")
            val utcDateTime = LocalDateTime.parse(item.timestamp, formatter)
            val zonedDateTime = utcDateTime.atZone(ZoneOffset.UTC)
            val localDateTime = zonedDateTime.withZoneSameInstant(ZoneId.systemDefault())
            localDateTime.format(DateTimeFormatter.ofPattern("MMM dd, HH:mm"))
        } catch (e: Exception) {
            item.timestamp
        }
    }

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
            Text(
                text = item.content,
                style = MaterialTheme.typography.bodyLarge,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis
            )
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

