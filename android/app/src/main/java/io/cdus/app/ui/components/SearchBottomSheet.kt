package io.cdus.app.ui.components

import android.widget.Toast
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
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
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.material.icons.filled.ContentPaste
import androidx.compose.material.icons.filled.Image
import androidx.compose.material.icons.filled.Description
import androidx.compose.material.icons.filled.Devices
import androidx.compose.material.icons.filled.HelpOutline
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import uniffi.cdus_ffi.search
import uniffi.cdus_ffi.FfiSearchResult

data class SearchItem(
    val id: String,
    val type: String, // "clipboard", "file", "device"
    val title: String,
    val subtitle: String,
    val icon: ImageVector
)

fun FfiSearchResult.toSearchItem(): SearchItem {
    val icon = when (itemType) {
        "clipboard" -> Icons.Default.ContentPaste
        "file" -> {
            if (title.endsWith(".png", ignoreCase = true) ||
                title.endsWith(".jpg", ignoreCase = true) ||
                title.endsWith(".jpeg", ignoreCase = true) ||
                title.endsWith(".gif", ignoreCase = true)) {
                Icons.Default.Image
            } else {
                Icons.Default.Description
            }
        }
        "device" -> Icons.Default.Devices
        else -> Icons.Default.HelpOutline
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
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        containerColor = MaterialTheme.colorScheme.surface
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 20.dp)
                .padding(bottom = 32.dp)
        ) {
            Column(modifier = Modifier.padding(bottom = 12.dp)) {
                Text(
                    text = "Mesh Search",
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.Bold
                )
                Text(
                    text = "Query paired devices, clipboard history, and shared transfers",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.outline
                )
            }

            OutlinedTextField(
                value = query,
                onValueChange = { query = it },
                placeholder = {
                    Text(
                        "Search clipboard, files, or peers...",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.outline
                    )
                },
                leadingIcon = {
                    Icon(
                        Icons.Default.Search,
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.primary
                    )
                },
                trailingIcon = {
                    if (query.isNotEmpty()) {
                        IconButton(
                            onClick = { query = "" },
                            modifier = Modifier.size(48.dp)
                        ) {
                            Icon(Icons.Default.Close, contentDescription = "Clear query")
                        }
                    }
                },
                shape = RoundedCornerShape(12.dp),
                colors = OutlinedTextFieldDefaults.colors(
                    focusedBorderColor = MaterialTheme.colorScheme.primary,
                    unfocusedBorderColor = MaterialTheme.colorScheme.outlineVariant
                ),
                modifier = Modifier.fillMaxWidth(),
                singleLine = true
            )

            Spacer(modifier = Modifier.height(16.dp))

            when (val state = screenState) {
                is SearchScreenState.Loading -> {
                    Box(
                        modifier = Modifier
                            .fillMaxWidth()
                            .height(180.dp),
                        contentAlignment = Alignment.Center
                    ) {
                        CircularProgressIndicator(
                            color = MaterialTheme.colorScheme.primary,
                            strokeWidth = 3.dp
                        )
                    }
                }
                is SearchScreenState.Empty -> {
                    Box(
                        modifier = Modifier
                            .fillMaxWidth()
                            .height(180.dp)
                            .padding(16.dp),
                        contentAlignment = Alignment.Center
                    ) {
                        Column(horizontalAlignment = Alignment.CenterHorizontally) {
                            Text(
                                text = if (query.isEmpty()) "Start typing to search" else "No matching records found",
                                style = MaterialTheme.typography.titleSmall,
                                fontWeight = FontWeight.SemiBold
                            )
                            Spacer(Modifier.height(4.dp))
                            Text(
                                text = if (query.isEmpty()) 
                                    "Search by text snippet, filename, extension (.pdf, .png), or peer label."
                                else 
                                    "No items matched \"$query\". Check spelling or search by file extension.",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.outline,
                                textAlign = androidx.compose.ui.text.style.TextAlign.Center
                            )
                        }
                    }
                }
                is SearchScreenState.Error -> {
                    Box(
                        modifier = Modifier
                            .fillMaxWidth()
                            .height(180.dp)
                            .padding(16.dp),
                        contentAlignment = Alignment.Center
                    ) {
                        Column(horizontalAlignment = Alignment.CenterHorizontally) {
                            Text(
                                text = "Search failed",
                                style = MaterialTheme.typography.titleSmall,
                                color = MaterialTheme.colorScheme.error,
                                fontWeight = FontWeight.Bold
                            )
                            Spacer(Modifier.height(4.dp))
                            Text(
                                text = state.message,
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.outline
                            )
                        }
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
                                    "clipboard" -> "Clipboard Records"
                                    "file" -> "File Transfers"
                                    "device" -> "Paired Devices"
                                    else -> "Other"
                                }
                                Text(
                                    text = groupTitle.uppercase(),
                                    style = MaterialTheme.typography.labelSmall,
                                    fontWeight = FontWeight.Bold,
                                    color = MaterialTheme.colorScheme.primary,
                                    letterSpacing = 1.sp,
                                    modifier = Modifier.padding(top = 8.dp, bottom = 4.dp)
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
                                    shape = RoundedCornerShape(10.dp),
                                    colors = CardDefaults.cardColors(
                                        containerColor = MaterialTheme.colorScheme.surface
                                    ),
                                    border = BorderStroke(
                                        1.dp,
                                        MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.5f)
                                    )
                                ) {
                                    Row(
                                        modifier = Modifier
                                            .fillMaxWidth()
                                            .padding(12.dp),
                                        verticalAlignment = Alignment.CenterVertically
                                    ) {
                                        Surface(
                                            shape = RoundedCornerShape(8.dp),
                                            color = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.5f),
                                            modifier = Modifier.size(40.dp)
                                        ) {
                                            Box(contentAlignment = Alignment.Center) {
                                                Icon(
                                                    imageVector = item.icon,
                                                    contentDescription = item.type,
                                                    tint = MaterialTheme.colorScheme.primary,
                                                    modifier = Modifier.size(22.dp)
                                                )
                                            }
                                        }
                                        Spacer(Modifier.width(12.dp))
                                        Column(modifier = Modifier.weight(1f)) {
                                            Text(
                                                text = item.title,
                                                style = MaterialTheme.typography.bodyMedium,
                                                fontWeight = FontWeight.SemiBold,
                                                maxLines = 1,
                                                overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis
                                            )
                                            Text(
                                                text = item.subtitle,
                                                style = MaterialTheme.typography.bodySmall,
                                                color = MaterialTheme.colorScheme.outline,
                                                maxLines = 1,
                                                overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis
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
