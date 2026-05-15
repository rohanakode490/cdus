package io.cdus.app.utils

import android.util.Log
import io.cdus.app.BuildConfig

object Logger {
    private const val TAG = "CDUS"

    fun d(message: String) {
        if (BuildConfig.DEBUG) {
            Log.d(TAG, message)
        }
    }

    fun i(message: String) {
        if (BuildConfig.DEBUG) {
            Log.i(TAG, message)
        }
    }

    fun w(message: String) {
        Log.w(TAG, message)
    }

    fun e(message: String) {
        Log.e(TAG, message)
    }

    fun e(message: String, throwable: Throwable) {
        Log.e(TAG, message, throwable)
    }
}
