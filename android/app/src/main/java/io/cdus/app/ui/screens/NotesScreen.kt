package io.cdus.app.ui.screens

import android.widget.Toast
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Description
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.cdus.app.ui.theme.*
import io.cdus.app.utils.Logger
import kotlinx.coroutines.*
import uniffi.cdus_ffi.FfiNote
import uniffi.cdus_ffi.NoteListener
import uniffi.cdus_ffi.deleteNote
import uniffi.cdus_ffi.getNotes
import uniffi.cdus_ffi.saveNote
import uniffi.cdus_ffi.setNoteListener
import java.text.SimpleDateFormat
import java.util.*

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun NotesScreen() {
    val context = LocalContext.current
    val coroutineScope = rememberCoroutineScope()

    var notesList by remember { mutableStateOf<List<FfiNote>>(emptyList()) }
    var searchQuery by remember { mutableStateOf("") }
    var isLoading by remember { mutableStateOf(true) }
    var selectedNote by remember { mutableStateOf<FfiNote?>(null) }
    var showEditor by remember { mutableStateOf(false) }

    // Load notes function
    fun loadNotes() {
        coroutineScope.launch {
            try {
                val freshNotes = withContext(Dispatchers.IO) {
                    getNotes().sortedByDescending { it.updatedAt }
                }
                notesList = freshNotes
            } catch (e: Exception) {
                Logger.e("Failed to load notes: ${e.message}")
            } finally {
                isLoading = false
            }
        }
    }

    // Set up NoteListener and initial load
    DisposableEffect(Unit) {
        val listener = object : NoteListener {
            override fun onNoteUpdated(note: FfiNote) {
                coroutineScope.launch(Dispatchers.Main) {
                    val updated = notesList.toMutableList()
                    val idx = updated.indexOfFirst { it.docId == note.docId }
                    if (idx >= 0) {
                        updated[idx] = note
                    } else {
                        updated.add(0, note)
                    }
                    notesList = updated.sortedByDescending { it.updatedAt }

                    // If editor is open with this note, update selectedNote if not editing
                    if (selectedNote?.docId == note.docId) {
                        selectedNote = note
                    }
                }
            }

            override fun onNoteDeleted(docId: String) {
                coroutineScope.launch(Dispatchers.Main) {
                    notesList = notesList.filter { it.docId !== docId }
                    if (selectedNote?.docId == docId) {
                        showEditor = false
                        selectedNote = null
                    }
                }
            }
        }

        try {
            setNoteListener(listener)
        } catch (e: Exception) {
            Logger.e("Error setting NoteListener: ${e.message}")
        }

        loadNotes()

        onDispose {
            // Cleanup if needed
        }
    }

    val filteredNotes = remember(notesList, searchQuery) {
        if (searchQuery.isBlank()) {
            notesList
        } else {
            val q = searchQuery.lowercase(Locale.ROOT).trim()
            notesList.filter {
                it.title.lowercase(Locale.ROOT).contains(q) ||
                it.content.lowercase(Locale.ROOT).contains(q)
            }
        }
    }

    Scaffold(
        topBar = {
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .background(MaterialTheme.colorScheme.background)
                    .padding(horizontal = 20.dp, vertical = 12.dp)
            ) {
                Text(
                    text = "Collaborative Notes",
                    style = MaterialTheme.typography.titleLarge,
                    fontWeight = FontWeight.Bold,
                    color = MaterialTheme.colorScheme.onBackground
                )
                Text(
                    text = "Decentralized document sync via Automerge CRDT",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(top = 2.dp)
                )

                Spacer(modifier = Modifier.height(12.dp))

                OutlinedTextField(
                    value = searchQuery,
                    onValueChange = { searchQuery = it },
                    placeholder = {
                        Text(
                            "Search notes...",
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    },
                    leadingIcon = {
                        Icon(
                            Icons.Default.Search,
                            contentDescription = "Search",
                            tint = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    },
                    trailingIcon = {
                        if (searchQuery.isNotEmpty()) {
                            IconButton(onClick = { searchQuery = "" }) {
                                Icon(
                                    Icons.Default.Close,
                                    contentDescription = "Clear",
                                    tint = MaterialTheme.colorScheme.onSurfaceVariant
                                )
                            }
                        }
                    },
                    modifier = Modifier
                        .fillMaxWidth()
                        .height(52.dp),
                    shape = RoundedCornerShape(10.dp),
                    colors = OutlinedTextFieldDefaults.colors(
                        focusedBorderColor = CdusCyan,
                        unfocusedBorderColor = MaterialTheme.colorScheme.outlineVariant,
                        focusedContainerColor = MaterialTheme.colorScheme.surface,
                        unfocusedContainerColor = MaterialTheme.colorScheme.surface
                    ),
                    singleLine = true
                )
            }
        },
        floatingActionButton = {
            FloatingActionButton(
                onClick = {
                    val newDocId = UUID.randomUUID().toString()
                    val now = System.currentTimeMillis().toULong()
                    val newNote = FfiNote(
                        docId = newDocId,
                        title = "Untitled Note",
                        content = "",
                        createdAt = now,
                        updatedAt = now
                    )
                    coroutineScope.launch(Dispatchers.IO) {
                        try {
                            saveNote(newDocId, newNote.title, newNote.content)
                        } catch (e: Exception) {
                            Logger.e("Failed to create note: ${e.message}")
                        }
                    }
                    selectedNote = newNote
                    showEditor = true
                },
                containerColor = CdusCyan,
                contentColor = MaterialTheme.colorScheme.background,
                shape = CircleShape
            ) {
                Icon(Icons.Default.Add, contentDescription = "New Note")
            }
        }
    ) { paddingValues ->
        Box(
            modifier = Modifier
                .fillMaxSize()
                .padding(paddingValues)
        ) {
            if (isLoading) {
                CircularProgressIndicator(
                    modifier = Modifier.align(Alignment.Center),
                    color = CdusCyan
                )
            } else if (filteredNotes.isEmpty()) {
                Column(
                    modifier = Modifier
                        .align(Alignment.Center)
                        .padding(32.dp),
                    horizontalAlignment = Alignment.CenterHorizontally
                ) {
                    Icon(
                        Icons.Default.Description,
                        contentDescription = null,
                        modifier = Modifier.size(56.dp),
                        tint = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.5f)
                    )
                    Spacer(modifier = Modifier.height(16.dp))
                    Text(
                        text = if (searchQuery.isNotEmpty()) "No matching notes found." else "No notes yet.",
                        style = MaterialTheme.typography.titleMedium,
                        color = MaterialTheme.colorScheme.onBackground
                    )
                    Text(
                        text = if (searchQuery.isNotEmpty()) "Try a different search query." else "Tap + below to create your first synchronized note.",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(top = 4.dp)
                    )
                }
            } else {
                LazyColumn(
                    modifier = Modifier.fillMaxSize(),
                    contentPadding = PaddingValues(horizontal = 20.dp, vertical = 12.dp),
                    verticalArrangement = Arrangement.spacedBy(10.dp)
                ) {
                    items(filteredNotes, key = { it.docId }) { note ->
                        NoteCard(
                            note = note,
                            onClick = {
                                selectedNote = note
                                showEditor = true
                            },
                            onDelete = {
                                coroutineScope.launch(Dispatchers.IO) {
                                    try {
                                        deleteNote(note.docId)
                                    } catch (e: Exception) {
                                        Logger.e("Failed to delete note: ${e.message}")
                                    }
                                }
                            }
                        )
                    }
                }
            }
        }
    }

    if (showEditor && selectedNote != null) {
        NoteEditorBottomSheet(
            note = selectedNote!!,
            onDismiss = {
                showEditor = false
                selectedNote = null
                loadNotes()
            },
            onDelete = {
                val docId = selectedNote!!.docId
                coroutineScope.launch(Dispatchers.IO) {
                    try {
                        deleteNote(docId)
                    } catch (e: Exception) {
                        Logger.e("Failed to delete note: ${e.message}")
                    }
                }
                showEditor = false
                selectedNote = null
                loadNotes()
            }
        )
    }
}

@Composable
fun NoteCard(
    note: FfiNote,
    onClick: () -> Unit,
    onDelete: () -> Unit
) {
    var showDeleteConfirm by remember { mutableStateOf(false) }

    val formattedDate = remember(note.updatedAt) {
        try {
            val sdf = SimpleDateFormat("MMM d, HH:mm", Locale.getDefault())
            sdf.format(Date(note.updatedAt.toLong()))
        } catch (e: Exception) {
            ""
        }
    }

    val snippet = remember(note.content) {
        if (note.content.isBlank()) {
            "No content yet"
        } else {
            note.content.trim().replace("\n", " ")
        }
    }

    Card(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .border(
                1.dp,
                MaterialTheme.colorScheme.outlineVariant,
                RoundedCornerShape(12.dp)
            ),
        shape = RoundedCornerShape(12.dp),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surface
        )
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(16.dp)
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically
            ) {
                Text(
                    text = if (note.title.isBlank()) "Untitled Note" else note.title,
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.SemiBold,
                    color = MaterialTheme.colorScheme.onSurface,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f)
                )

                IconButton(
                    onClick = { showDeleteConfirm = true },
                    modifier = Modifier.size(28.dp)
                ) {
                    Icon(
                        Icons.Default.Delete,
                        contentDescription = "Delete",
                        tint = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.6f),
                        modifier = Modifier.size(18.dp)
                    )
                }
            }

            Spacer(modifier = Modifier.height(4.dp))

            Text(
                text = snippet,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis
            )

            Spacer(modifier = Modifier.height(10.dp))

            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically
            ) {
                Text(
                    text = formattedDate,
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.outline
                )

                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(4.dp)
                ) {
                    Box(
                        modifier = Modifier
                            .size(6.dp)
                            .clip(CircleShape)
                            .background(CdusStatusOnline)
                    )
                    Text(
                        text = "CRDT Synced",
                        style = MaterialTheme.typography.labelSmall,
                        color = CdusStatusOnline
                    )
                }
            }
        }
    }

    if (showDeleteConfirm) {
        AlertDialog(
            onDismissRequest = { showDeleteConfirm = false },
            title = { Text("Delete Note") },
            text = { Text("Are you sure you want to delete '${if (note.title.isBlank()) "Untitled Note" else note.title}'? This change will sync across all paired devices.") },
            confirmButton = {
                TextButton(
                    onClick = {
                        showDeleteConfirm = false
                        onDelete()
                    }
                ) {
                    Text("Delete", color = CdusStatusOffline)
                }
            },
            dismissButton = {
                TextButton(onClick = { showDeleteConfirm = false }) {
                    Text("Cancel")
                }
            }
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun NoteEditorBottomSheet(
    note: FfiNote,
    onDismiss: () -> Unit,
    onDelete: () -> Unit
) {
    val coroutineScope = rememberCoroutineScope()
    var title by remember { mutableStateOf(note.title) }
    var content by remember { mutableStateOf(note.content) }
    var isSyncing by remember { mutableStateOf(false) }
    var saveJob by remember { mutableStateOf<Job?>(null) }
    var showDeleteConfirm by remember { mutableStateOf(false) }

    fun scheduleSave() {
        isSyncing = true
        saveJob?.cancel()
        saveJob = coroutineScope.launch {
            delay(400) // Debounce auto-save
            try {
                withContext(Dispatchers.IO) {
                    saveNote(note.docId, title, content)
                }
                isSyncing = false
            } catch (e: Exception) {
                Logger.e("Error auto-saving note: ${e.message}")
                isSyncing = false
            }
        }
    }

    ModalBottomSheet(
        onDismissRequest = {
            // Ensure any pending save is executed immediately on dismiss
            saveJob?.cancel()
            coroutineScope.launch(Dispatchers.IO) {
                try {
                    saveNote(note.docId, title, content)
                } catch (e: Exception) {
                    Logger.e("Error saving note on close: ${e.message}")
                }
            }
            onDismiss()
        },
        containerColor = MaterialTheme.colorScheme.surface,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        modifier = Modifier.fillMaxHeight(0.92f)
    ) {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(horizontal = 20.dp, vertical = 8.dp)
        ) {
            // Header Bar
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically
            ) {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(6.dp)
                ) {
                    Box(
                        modifier = Modifier
                            .size(8.dp)
                            .clip(CircleShape)
                            .background(if (isSyncing) CdusCyan else CdusStatusOnline)
                    )
                    Text(
                        text = if (isSyncing) "Syncing..." else "Synced",
                        style = MaterialTheme.typography.labelSmall,
                        color = if (isSyncing) CdusCyan else CdusStatusOnline,
                        fontWeight = FontWeight.Medium
                    )
                }

                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp)
                ) {
                    IconButton(onClick = { showDeleteConfirm = true }) {
                        Icon(
                            Icons.Default.Delete,
                            contentDescription = "Delete Note",
                            tint = CdusStatusOffline
                        )
                    }

                    TextButton(
                        onClick = {
                            saveJob?.cancel()
                            coroutineScope.launch(Dispatchers.IO) {
                                try {
                                    saveNote(note.docId, title, content)
                                } catch (e: Exception) {
                                    Logger.e("Error saving note on close: ${e.message}")
                                }
                            }
                            onDismiss()
                        }
                    ) {
                        Text(
                            "Done",
                            color = CdusCyan,
                            fontWeight = FontWeight.SemiBold
                        )
                    }
                }
            }

            Spacer(modifier = Modifier.height(16.dp))

            // Title Input (Borderless)
            TextField(
                value = title,
                onValueChange = {
                    title = it
                    scheduleSave()
                },
                placeholder = {
                    Text(
                        "Title",
                        style = MaterialTheme.typography.titleLarge.copy(fontSize = 22.sp),
                        fontWeight = FontWeight.Bold,
                        color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.5f)
                    )
                },
                modifier = Modifier.fillMaxWidth(),
                textStyle = MaterialTheme.typography.titleLarge.copy(
                    fontSize = 22.sp,
                    fontWeight = FontWeight.Bold,
                    color = MaterialTheme.colorScheme.onSurface
                ),
                colors = TextFieldDefaults.colors(
                    focusedContainerColor = MaterialTheme.colorScheme.surface,
                    unfocusedContainerColor = MaterialTheme.colorScheme.surface,
                    focusedIndicatorColor = androidx.compose.ui.graphics.Color.Transparent,
                    unfocusedIndicatorColor = androidx.compose.ui.graphics.Color.Transparent
                ),
                singleLine = true
            )

            HorizontalDivider(
                color = MaterialTheme.colorScheme.outlineVariant,
                modifier = Modifier.padding(vertical = 8.dp)
            )

            // Content Input (Borderless, Multiline)
            TextField(
                value = content,
                onValueChange = {
                    content = it
                    scheduleSave()
                },
                placeholder = {
                    Text(
                        "Start typing... Changes converge across all devices instantly.",
                        style = MaterialTheme.typography.bodyLarge,
                        color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.5f)
                    )
                },
                modifier = Modifier
                    .fillMaxWidth()
                    .weight(1f),
                textStyle = MaterialTheme.typography.bodyLarge.copy(
                    lineHeight = 24.sp,
                    color = MaterialTheme.colorScheme.onSurface
                ),
                colors = TextFieldDefaults.colors(
                    focusedContainerColor = MaterialTheme.colorScheme.surface,
                    unfocusedContainerColor = MaterialTheme.colorScheme.surface,
                    focusedIndicatorColor = androidx.compose.ui.graphics.Color.Transparent,
                    unfocusedIndicatorColor = androidx.compose.ui.graphics.Color.Transparent
                )
            )
        }
    }

    if (showDeleteConfirm) {
        AlertDialog(
            onDismissRequest = { showDeleteConfirm = false },
            title = { Text("Delete Note") },
            text = { Text("Are you sure you want to delete this note? This action cannot be undone.") },
            confirmButton = {
                TextButton(
                    onClick = {
                        showDeleteConfirm = false
                        onDelete()
                    }
                ) {
                    Text("Delete", color = CdusStatusOffline)
                }
            },
            dismissButton = {
                TextButton(onClick = { showDeleteConfirm = false }) {
                    Text("Cancel")
                }
            }
        )
    }
}
