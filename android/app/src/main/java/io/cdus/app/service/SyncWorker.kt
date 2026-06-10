package io.cdus.app.service

import android.content.Context
import android.content.Intent
import androidx.core.content.ContextCompat
import androidx.work.CoroutineWorker
import androidx.work.WorkerParameters
import io.cdus.app.CoreInitializer
import io.cdus.app.utils.Logger
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext

class SyncWorker(
    context: Context,
    params: WorkerParameters
) : CoroutineWorker(context, params) {

    override suspend fun doWork(): Result = withContext(Dispatchers.IO) {
        Logger.i("SyncWorker: Work started")
        val context = applicationContext
        
        try {
            val sharedPref = context.getSharedPreferences("cdus_settings", Context.MODE_PRIVATE)
            val clipboardSyncEnabled = sharedPref.getBoolean("clipboard_sync", false)

            if (clipboardSyncEnabled) {
                // Clipboard sync is enabled, so SyncService should be running.
                if (!SyncService.isRunning) {
                    Logger.i("SyncWorker: SyncService is enabled but not running, starting it...")
                    val intent = Intent(context, SyncService::class.java)
                    ContextCompat.startForegroundService(context, intent)
                } else {
                    Logger.d("SyncWorker: SyncService is already running")
                }
            } else {
                // Clipboard sync is disabled, but we run a short background sync session
                // for peer discovery and general file sync queues.
                Logger.i("SyncWorker: Starting a temporary sync session...")
                CoreInitializer.initialize(context)
                
                // Let the background threads run for 60 seconds to process any pending sync items.
                delay(60000)
                
                // Only clean up if SyncService is still not running.
                if (!SyncService.isRunning) {
                    Logger.i("SyncWorker: Temporary sync session finished, cleaning up...")
                    CoreInitializer.cleanup()
                } else {
                    Logger.d("SyncWorker: SyncService was started during sync, skipping cleanup")
                }
            }
            Result.success()
        } catch (e: Exception) {
            Logger.e("SyncWorker: Error during background sync: ${e.message}")
            Result.retry()
        }
    }
}
