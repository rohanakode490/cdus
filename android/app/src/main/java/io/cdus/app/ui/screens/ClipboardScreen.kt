package io.cdus.app.ui.screens

import android.widget.Toast
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

data class ClipboardItem(
    val id: String,
    val content: String,
    val sourceDevice: String,
    val timestamp: String
)

private val mockClipboardHistory = listOf(
    ClipboardItem("1", "https://github.com/google/gemini-cli", "MacBook Pro", "2 min ago"),
    ClipboardItem("2", "Gemini is a family of multimodal AI models.", "Linux Desktop", "15 min ago"),
    ClipboardItem("3", "TODO: Buy milk and bread", "Pixel 7", "1 hour ago"),
    ClipboardItem("4", "ssh user@192.168.1.10", "MacBook Pro", "3 hours ago"),
    ClipboardItem("5", "package main\n\nfunc main() {\n\tprintln(\"Hello\")\n}", "Linux Desktop", "5 hours ago"),
    ClipboardItem("6", "https://tauri.app/v2", "Pixel 7", "Yesterday"),
    ClipboardItem("7", "Meeting at 3 PM today", "MacBook Pro", "Yesterday"),
    ClipboardItem("8", "API_KEY=sk_test_placeholder_key_value", "Linux Desktop", "2 days ago"),
    ClipboardItem("9", "Jetpack Compose Navigation Guide", "Pixel 7", "3 days ago"),
    ClipboardItem("10", "Color(0xFF6200EE)", "MacBook Pro", "5 days ago")
)

@Composable
fun ClipboardScreen() {
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

        LazyColumn(
            modifier = Modifier.fillMaxSize(),
            verticalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            items(mockClipboardHistory) { item ->
                ClipboardListItem(item)
            }
        }
    }
}

@Composable
fun ClipboardListItem(item: ClipboardItem) {
    val clipboardManager = LocalClipboardManager.current
    val context = LocalContext.current

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
                    text = "from ${item.sourceDevice}",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.secondary
                )
                Text(
                    text = item.timestamp,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.outline
                )
            }
        }
    }
}

