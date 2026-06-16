package io.cdus.app.ui.components

import android.widget.Toast
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import uniffi.cdus_ffi.search
import uniffi.cdus_ffi.FfiSearchResult

data class SearchItem(
    val id: String,
    val type: String, // "clipboard", "file", "device"
    val title: String,
    val subtitle: String,
    val icon: String
)

fun FfiSearchResult.toSearchItem(): SearchItem {
    val icon = when (itemType) {
        "clipboard" -> "📋"
        "file" -> {
            if (title.endsWith(".png", ignoreCase = true) ||
                title.endsWith(".jpg", ignoreCase = true) ||
                title.endsWith(".jpeg", ignoreCase = true) ||
                title.endsWith(".gif", ignoreCase = true)) {
                "🖼️"
            } else {
                "📄"
            }
        }
        "device" -> "💻"
        else -> "❓"
    }
    return SearchItem(
        id = id,
        type = itemType,
        title = title,
        subtitle = subtitle,
        icon = icon
    )
}

sealed class SearchScreenState {
    object Loading : SearchScreenState()
    data class Success(val results: List<SearchItem>) : SearchScreenState()
    data class Error(val message: String) : SearchScreenState()
    object Empty : SearchScreenState()
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SearchBottomSheet(
    onDismiss: () -> Unit,
    onNavigateToDevices: () -> Unit
) {
    var query by remember { mutableStateOf("") }
    var screenState by remember { mutableStateOf<SearchScreenState>(SearchScreenState.Loading) }
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    val clipboardManager = LocalClipboardManager.current

    // Debounced search logic querying the real Rust SQLite index
    LaunchedEffect(query) {
        screenState = SearchScreenState.Loading
        if (query.isNotEmpty()) {
            delay(150)
        }
        try {
            val rawResults = withContext(Dispatchers.IO) {
                search(query)
            }
            val mappedResults = rawResults.map { it.toSearchItem() }
            if (mappedResults.isEmpty()) {
                screenState = SearchScreenState.Empty
            } else {
                screenState = SearchScreenState.Success(mappedResults)
            }
        } catch (e: Exception) {
            screenState = SearchScreenState.Error(e.message ?: "Search failed")
        }
    }

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp)
                .padding(bottom = 32.dp)
        ) {
            Text(
                text = "Search Everything",
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.Bold,
                modifier = Modifier.padding(bottom = 8.dp)
            )

            OutlinedTextField(
                value = query,
                onValueChange = { query = it },
                placeholder = { Text("Search clipboard, files, devices...") },
                leadingIcon = { Icon(Icons.Default.Search, contentDescription = null) },
                trailingIcon = {
                    if (query.isNotEmpty()) {
                        IconButton(onClick = { query = "" }) {
                            Icon(Icons.Default.Close, contentDescription = "Clear query")
                        }
                    }
                },
                modifier = Modifier.fillMaxWidth(),
                singleLine = true
            )

            Spacer(modifier = Modifier.height(16.dp))

            when (val state = screenState) {
                is SearchScreenState.Loading -> {
                    Box(
                        modifier = Modifier
                            .fillMaxWidth()
                            .height(200.dp),
                        contentAlignment = Alignment.Center
                    ) {
                        CircularProgressIndicator()
                    }
                }
                is SearchScreenState.Empty -> {
                    Box(
                        modifier = Modifier
                            .fillMaxWidth()
                            .height(200.dp),
                        contentAlignment = Alignment.Center
                    ) {
                        Text(
                            text = "No matching results found.",
                            color = MaterialTheme.colorScheme.outline
                        )
                    }
                }
                is SearchScreenState.Error -> {
                    Box(
                        modifier = Modifier
                            .fillMaxWidth()
                            .height(200.dp),
                        contentAlignment = Alignment.Center
                    ) {
                        Text(
                            text = state.message,
                            color = MaterialTheme.colorScheme.error
                        )
                    }
                }
                is SearchScreenState.Success -> {
                    LazyColumn(
                        modifier = Modifier
                            .fillMaxWidth()
                            .weight(1f, fill = false),
                        verticalArrangement = Arrangement.spacedBy(8.dp)
                    ) {
                        val grouped = state.results.groupBy { it.type }

                        grouped.forEach { (type, items) ->
                            item {
                                val groupTitle = when (type) {
                                    "clipboard" -> "Clipboard History"
                                    "file" -> "Files"
                                    "device" -> "Devices"
                                    else -> "Other"
                                }
                                Text(
                                    text = groupTitle.uppercase(),
                                    fontSize = 11.sp,
                                    fontWeight = FontWeight.Bold,
                                    color = MaterialTheme.colorScheme.primary,
                                    modifier = Modifier.padding(vertical = 4.dp)
                                )
                            }

                            items(items) { item ->
                                Card(
                                    modifier = Modifier
                                        .fillMaxWidth()
                                        .clickable {
                                            when (item.type) {
                                                "clipboard" -> {
                                                    clipboardManager.setText(AnnotatedString(item.title))
                                                    Toast
                                                        .makeText(
                                                            context,
                                                            "Copied to clipboard",
                                                            Toast.LENGTH_SHORT
                                                        )
                                                        .show()
                                                    onDismiss()
                                                }
                                                "file" -> {
                                                    Toast
                                                        .makeText(
                                                            context,
                                                            "Opening file: ${item.title}",
                                                            Toast.LENGTH_SHORT
                                                        )
                                                        .show()
                                                    onDismiss()
                                                }
                                                "device" -> {
                                                    onNavigateToDevices()
                                                    onDismiss()
                                                }
                                            }
                                        },
                                    colors = CardDefaults.cardColors(
                                        containerColor = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.5f)
                                    )
                                ) {
                                    Row(
                                        modifier = Modifier
                                            .fillMaxWidth()
                                            .padding(12.dp),
                                        verticalAlignment = Alignment.CenterVertically
                                    ) {
                                        Text(
                                            text = item.icon,
                                            fontSize = 24.sp,
                                            modifier = Modifier.padding(end = 12.dp)
                                        )
                                        Column(modifier = Modifier.weight(1f)) {
                                            Text(
                                                text = item.title,
                                                fontWeight = FontWeight.SemiBold,
                                                fontSize = 14.sp
                                            )
                                            Text(
                                                text = item.subtitle,
                                                fontSize = 12.sp,
                                                color = MaterialTheme.colorScheme.outline
                                            )
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
