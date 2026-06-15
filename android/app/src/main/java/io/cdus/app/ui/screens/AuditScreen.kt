package io.cdus.app.ui.screens

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.cdus_ffi.getAuditLogs
import uniffi.cdus_ffi.clearAuditLogs
import uniffi.cdus_ffi.AuditLogItem
import java.text.SimpleDateFormat
import java.util.*

sealed class AuditScreenState {
    object Loading : AuditScreenState()
    data class Success(val logs: List<AuditLogItem>) : AuditScreenState()
    data class Error(val message: String) : AuditScreenState()
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AuditScreen() {
    var screenState by remember { mutableStateOf<AuditScreenState>(AuditScreenState.Loading) }
    val scope = rememberCoroutineScope()

    fun loadLogs() {
        scope.launch {
            screenState = AuditScreenState.Loading
            try {
                val freshLogs = withContext(Dispatchers.IO) {
                    getAuditLogs(100u)
                }
                screenState = AuditScreenState.Success(freshLogs)
            } catch (e: Exception) {
                screenState = AuditScreenState.Error(e.message ?: "Failed to load audit logs")
            }
        }
    }

    LaunchedEffect(Unit) {
        while (isActive) {
            try {
                val freshLogs = withContext(Dispatchers.IO) {
                    getAuditLogs(100u)
                }
                screenState = AuditScreenState.Success(freshLogs)
            } catch (e: Exception) {
                if (screenState is AuditScreenState.Loading) {
                    screenState = AuditScreenState.Error(e.message ?: "Failed to load audit logs")
                }
            }
            delay(5000) // Poll every 5 seconds
        }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Audit Log", fontWeight = FontWeight.Bold) },
                actions = {
                    IconButton(onClick = { loadLogs() }) {
                        Icon(Icons.Default.Refresh, contentDescription = "Refresh")
                    }
                    IconButton(onClick = {
                        scope.launch(Dispatchers.IO) {
                            clearAuditLogs()
                            loadLogs()
                        }
                    }) {
                        Icon(Icons.Default.Delete, contentDescription = "Clear Logs", tint = MaterialTheme.colorScheme.error)
                    }
                }
            )
        }
    ) { innerPadding ->
        Box(
            modifier = Modifier
                .fillMaxSize()
                .padding(innerPadding)
        ) {
            when (val state = screenState) {
                is AuditScreenState.Loading -> {
                    Column(
                        modifier = Modifier.fillMaxSize(),
                        verticalArrangement = Arrangement.Center,
                        horizontalAlignment = Alignment.CenterHorizontally
                    ) {
                        CircularProgressIndicator()
                        Spacer(modifier = Modifier.height(16.dp))
                        Text(
                            text = "Loading audit logs...",
                            color = MaterialTheme.colorScheme.onBackground.copy(alpha = 0.6f)
                        )
                    }
                }
                is AuditScreenState.Error -> {
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
                            modifier = Modifier.size(48.dp)
                        )
                        Spacer(modifier = Modifier.height(16.dp))
                        Text(
                            text = state.message,
                            color = MaterialTheme.colorScheme.error,
                            fontWeight = FontWeight.Medium,
                            fontSize = 16.sp
                        )
                        Spacer(modifier = Modifier.height(16.dp))
                        Button(onClick = { loadLogs() }) {
                            Text("Retry")
                        }
                    }
                }
                is AuditScreenState.Success -> {
                    if (state.logs.isEmpty()) {
                        Column(
                            modifier = Modifier
                                .fillMaxSize()
                                .padding(24.dp),
                            verticalArrangement = Arrangement.Center,
                            horizontalAlignment = Alignment.CenterHorizontally
                        ) {
                            Text(
                                text = "No audit log entries found.",
                                color = MaterialTheme.colorScheme.onBackground.copy(alpha = 0.5f),
                                fontSize = 16.sp,
                                fontStyle = androidx.compose.ui.text.font.FontStyle.Italic
                            )
                            Spacer(modifier = Modifier.height(16.dp))
                            Button(onClick = { loadLogs() }) {
                                Text("Refresh")
                            }
                        }
                    } else {
                        LazyColumn(
                            modifier = Modifier
                                .fillMaxSize()
                                .padding(horizontal = 16.dp),
                            verticalArrangement = Arrangement.spacedBy(8.dp)
                        ) {
                            items(state.logs) { log ->
                                AuditLogItem(log)
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
fun AuditLogItem(log: AuditLogItem) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        shape = RoundedCornerShape(8.dp),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.5f)
        )
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(12.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    val badgeColor = when (log.eventType) {
                        "sync" -> Color(0xFF24C8DB)
                        "pairing" -> Color(0xFF5E35B1)
                        else -> Color(0xFF616161)
                    }
                    val badgeText = log.eventType.uppercase()
                    Surface(
                        color = badgeColor,
                        shape = RoundedCornerShape(4.dp),
                        modifier = Modifier.padding(end = 8.dp)
                    ) {
                        Text(
                            text = badgeText,
                            color = Color.White,
                            fontSize = 10.sp,
                            fontWeight = FontWeight.Bold,
                            modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp)
                        )
                    }
                }
                Spacer(Modifier.height(4.dp))
                Text(
                    text = log.content,
                    fontSize = 14.sp,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
            val timeStr = remember(log.timestamp) {
                SimpleDateFormat("yyyy-MM-dd HH:mm:ss", Locale.getDefault()).format(Date(log.timestamp.toLong()))
            }
            Text(
                text = timeStr,
                fontSize = 12.sp,
                color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.6f)
            )
        }
    }
}
