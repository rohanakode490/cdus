package io.cdus.app.utils

import android.content.ContentResolver
import android.content.ContentValues
import android.content.Context
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import android.provider.OpenableColumns
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
                put(MediaStore.MediaColumns.RELATIVE_PATH, Environment.DIRECTORY_DOWNLOADS)
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
}
