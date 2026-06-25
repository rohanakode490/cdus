package io.cdus.app.utils

import android.content.ContentResolver
import android.content.ContentValues
import android.content.Context
import android.content.ContentUris
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import android.provider.OpenableColumns
import android.webkit.MimeTypeMap
import android.widget.Toast
import java.io.File
import java.io.FileInputStream

object FileUtils {
    fun copyUriToLocal(context: Context, uri: Uri): String? {
        try {
            val contentResolver = context.contentResolver
            val fileName = getFileName(contentResolver, uri) ?: "shared_file"
            val tempFile = File(context.cacheDir, fileName)
            
            contentResolver.openInputStream(uri)?.use { input ->
                tempFile.outputStream().use { output ->
                    input.copyTo(output)
                }
            }
            return tempFile.absolutePath
        } catch (e: Exception) {
            Logger.e("Failed to copy URI to local: ${e.message}")
            return null
        }
    }

    fun getFileName(contentResolver: ContentResolver, uri: Uri): String? {
        var name: String? = null
        try {
            val cursor = contentResolver.query(uri, null, null, null, null)
            cursor?.use {
                if (it.moveToFirst()) {
                    val index = it.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                    if (index != -1) name = it.getString(index)
                }
            }
        } catch (e: Exception) {
            Logger.e("Error getting file name: ${e.message}")
        }
        return name ?: uri.lastPathSegment
    }

    fun saveFileToDownloads(context: Context, sourceFile: File): Uri? {
        val resolver = context.contentResolver
        val contentValues = ContentValues().apply {
            put(MediaStore.MediaColumns.DISPLAY_NAME, sourceFile.name)
            put(MediaStore.MediaColumns.MIME_TYPE, "*/*")
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                put(MediaStore.MediaColumns.RELATIVE_PATH, Environment.DIRECTORY_DOWNLOADS + java.io.File.separator + "cdus")
                put(MediaStore.MediaColumns.IS_PENDING, 1)
            }
        }

        val collection = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            MediaStore.Downloads.EXTERNAL_CONTENT_URI
        } else {
            @Suppress("DEPRECATION")
            Uri.parse("content://media/external/file") 
        }

        val uri = resolver.insert(collection, contentValues)
        uri?.let {
            try {
                resolver.openOutputStream(it)?.use { output ->
                    FileInputStream(sourceFile).use { input ->
                        input.copyTo(output)
                    }
                }
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                    contentValues.clear()
                    contentValues.put(MediaStore.MediaColumns.IS_PENDING, 0)
                    resolver.update(it, contentValues, null, null)
                }
            } catch (e: Exception) {
                Logger.e("Failed to save file to downloads: ${e.message}")
                return null
            }
        }
        return uri
    }

    fun deleteFileFromDownloads(context: Context, fileName: String): Boolean {
        return try {
            val resolver = context.contentResolver
            val collection = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                MediaStore.Downloads.EXTERNAL_CONTENT_URI
            } else {
                @Suppress("DEPRECATION")
                Uri.parse("content://media/external/file")
            }
            
            val projection = arrayOf(MediaStore.MediaColumns._ID)
            val selection = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                "${MediaStore.MediaColumns.DISPLAY_NAME} = ? AND ${MediaStore.MediaColumns.RELATIVE_PATH} = ?"
            } else {
                "${MediaStore.MediaColumns.DISPLAY_NAME} = ?"
            }
            val selectionArgs = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                arrayOf(fileName, Environment.DIRECTORY_DOWNLOADS + java.io.File.separator + "cdus" + java.io.File.separator)
            } else {
                arrayOf(fileName)
            }
            
            var deleted = false
            resolver.query(collection, projection, selection, selectionArgs, null)?.use { cursor ->
                if (cursor.moveToFirst()) {
                    val id = cursor.getLong(cursor.getColumnIndexOrThrow(MediaStore.MediaColumns._ID))
                    val fileUri = ContentUris.withAppendedId(collection, id)
                    resolver.delete(fileUri, null, null)
                    deleted = true
                }
            }
            deleted
        } catch (e: Exception) {
            Logger.e("Failed to delete file from downloads: ${e.message}")
            false
        }
    }

    fun openFile(context: Context, fileName: String) {
        try {
            val fileUri = findFileUriInDownloads(context, fileName)
            if (fileUri != null) {
                val extension = MimeTypeMap.getFileExtensionFromUrl(fileName) ?: ""
                val mimeType = MimeTypeMap.getSingleton().getMimeTypeFromExtension(extension.lowercase()) ?: "*/*"
                
                val intent = Intent(Intent.ACTION_VIEW).apply {
                    setDataAndType(fileUri, mimeType)
                    addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_GRANT_READ_URI_PERMISSION)
                }
                context.startActivity(intent)
            } else {
                Toast.makeText(context, "The file could not be found. It may have been moved or deleted.", Toast.LENGTH_SHORT).show()
            }
        } catch (e: Exception) {
            Logger.e("Failed to open file: ${e.message}", e)
            Toast.makeText(context, "Failed to open file: ${e.localizedMessage}", Toast.LENGTH_SHORT).show()
        }
    }

    fun openFileLocation(context: Context, fileName: String) {
        try {
            val uri = Uri.parse("content://com.android.externalstorage.documents/document/primary%3ADownload%2Fcdus")
            val intent = Intent(Intent.ACTION_VIEW).apply {
                setDataAndType(uri, "vnd.android.document/directory")
                addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }
            context.startActivity(intent)
        } catch (e: Exception) {
            Logger.w("Failed to open specific cdus folder: ${e.message}, falling back to Downloads folder")
            try {
                val intent = Intent(android.app.DownloadManager.ACTION_VIEW_DOWNLOADS).apply {
                    addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                }
                context.startActivity(intent)
            } catch (ex: Exception) {
                Logger.e("Failed to open Downloads folder: ${ex.message}", ex)
                Toast.makeText(context, "Could not open folder. Please check your Downloads folder.", Toast.LENGTH_SHORT).show()
            }
        }
    }

    private fun findFileUriInDownloads(context: Context, fileName: String): Uri? {
        val uriInCdus = findFileUriInDownloadsHelper(context, fileName, searchSubfolderOnly = true)
        if (uriInCdus != null) return uriInCdus

        return findFileUriInDownloadsHelper(context, fileName, searchSubfolderOnly = false)
    }

    private fun findFileUriInDownloadsHelper(context: Context, fileName: String, searchSubfolderOnly: Boolean): Uri? {
        return try {
            val resolver = context.contentResolver
            val collection = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                MediaStore.Downloads.EXTERNAL_CONTENT_URI
            } else {
                @Suppress("DEPRECATION")
                Uri.parse("content://media/external/file")
            }
            
            val projection = arrayOf(MediaStore.MediaColumns._ID)
            val selection: String
            val selectionArgs: Array<String>
            
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q && searchSubfolderOnly) {
                selection = "${MediaStore.MediaColumns.DISPLAY_NAME} = ? AND ${MediaStore.MediaColumns.RELATIVE_PATH} = ?"
                selectionArgs = arrayOf(fileName, Environment.DIRECTORY_DOWNLOADS + java.io.File.separator + "cdus" + java.io.File.separator)
            } else {
                selection = "${MediaStore.MediaColumns.DISPLAY_NAME} = ?"
                selectionArgs = arrayOf(fileName)
            }
            
            var fileUri: Uri? = null
            resolver.query(collection, projection, selection, selectionArgs, null)?.use { cursor ->
                if (cursor.moveToFirst()) {
                    val id = cursor.getLong(cursor.getColumnIndexOrThrow(MediaStore.MediaColumns._ID))
                    fileUri = ContentUris.withAppendedId(collection, id)
                }
            }
            fileUri
        } catch (e: Exception) {
            Logger.e("Failed to find file in downloads (subfolderOnly=$searchSubfolderOnly): ${e.message}", e)
            null
        }
    }
}
