package io.cdus.app

import android.app.Application
import androidx.work.Constraints
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.NetworkType
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import io.cdus.app.service.SyncWorker
import io.cdus.app.utils.Logger
import java.util.concurrent.TimeUnit

class CdusApplication : Application() {

    override fun onCreate() {
        super.onCreate()
        Logger.i("CdusApplication: App starting, scheduling background work...")
        scheduleBackgroundSync()
    }

    private fun scheduleBackgroundSync() {
        val constraints = Constraints.Builder()
            .setRequiredNetworkType(NetworkType.CONNECTED)
            .build()

        val syncRequest = PeriodicWorkRequestBuilder<SyncWorker>(
            15, TimeUnit.MINUTES, // WorkManager minimum interval is 15 minutes
            5, TimeUnit.MINUTES
        )
            .setConstraints(constraints)
            .build()

        WorkManager.getInstance(this).enqueueUniquePeriodicWork(
            "CDUS_BACKGROUND_SYNC",
            ExistingPeriodicWorkPolicy.KEEP,
            syncRequest
        )
        Logger.i("CdusApplication: Background sync worker scheduled successfully")
    }
}
