package io.cdus.app

import android.content.Context
import android.content.Intent
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.net.wifi.WifiManager
import android.os.Build
import androidx.core.content.ContextCompat
import io.cdus.app.data.FileTransferManager
import io.cdus.app.utils.Logger
import uniffi.cdus_ffi.initCore
import uniffi.cdus_ffi.initLogging
import uniffi.cdus_ffi.registerDevice

object CoreInitializer {
    private var isInitialized = false
    private var multicastLock: WifiManager.MulticastLock? = null
    private var nsdManager: NsdManager? = null
    private var registrationListener: NsdManager.RegistrationListener? = null

    @Synchronized
    fun initialize(context: Context) {
        if (isInitialized) {
            Logger.d("CoreInitializer: Core already initialized")
            return
        }

        try {
            initLogging()
            
            val appContext = context.applicationContext
            
            // Initialize Rust Core
            val dataDir = appContext.filesDir.absolutePath
            val deviceName = Build.MODEL
            val identity = initCore(dataDir, deviceName)
            if (!identity.startsWith("error:")) {
                val parts = identity.split(":", limit = 2)
                if (parts.size >= 2) {
                    val nodeId = parts[0]
                    val label = parts[1]
                    val port = 5200
                    registerDevice(nodeId, label, port.toUShort())
                    Logger.i("CoreInitializer: Device registered in Rust: $nodeId ($label)")

                    // Native mDNS Service registration
                    registerNativeMdnsService(appContext, nodeId, label, port)

                    // Load file transfer history
                    FileTransferManager.loadHistory()

                    isInitialized = true
                    Logger.i("CoreInitializer: Core initialized successfully")
                } else {
                    Logger.e("CoreInitializer: Unexpected identity format: $identity")
                }
            } else {
                Logger.e("CoreInitializer: Failed to init core: $identity")
            }
        } catch (e: Exception) {
            Logger.e("CoreInitializer: Exception during initialization: ${e.message}")
        }
    }

    private fun registerNativeMdnsService(context: Context, nodeId: String, label: String, port: Int) {
        nsdManager = context.getSystemService(Context.NSD_SERVICE) as NsdManager

        val serviceInfo = NsdServiceInfo().apply {
            serviceName = if (nodeId.length > 32) nodeId.substring(0, 32) else nodeId
            serviceType = "_cdus._tcp."
            setPort(port)
            setAttribute("node_id", nodeId)
            setAttribute("label", label)
            setAttribute("os", "Android")
        }

        registrationListener = object : NsdManager.RegistrationListener {
            override fun onServiceRegistered(nsdServiceInfo: NsdServiceInfo) {
                Logger.i("CoreInitializer: Native Android mDNS registered successfully: ${nsdServiceInfo.serviceName}")
            }

            override fun onRegistrationFailed(serviceInfo: NsdServiceInfo, errorCode: Int) {
                Logger.e("CoreInitializer: Native Android mDNS registration failed: Error code $errorCode")
            }

            override fun onServiceUnregistered(arg0: NsdServiceInfo) {
                Logger.i("CoreInitializer: Native Android mDNS service unregistered.")
            }

            override fun onUnregistrationFailed(serviceInfo: NsdServiceInfo, errorCode: Int) {
                Logger.e("CoreInitializer: Native Android mDNS unregistration failed: Error code $errorCode")
            }
        }

        try {
            nsdManager?.registerService(
                serviceInfo, NsdManager.PROTOCOL_DNS_SD, registrationListener
            )
            Logger.i("CoreInitializer: Requested native Android mDNS registration")
        } catch (e: Exception) {
            Logger.e("CoreInitializer: Exception during native mDNS registration: ${e.message}")
        }
    }

    @Synchronized
    fun cleanup() {
        if (!isInitialized) return
        
        registrationListener?.let {
            try {
                nsdManager?.unregisterService(it)
                Logger.i("CoreInitializer: Unregistered native mDNS service")
            } catch (e: Exception) {
                Logger.e("CoreInitializer: Error unregistering NSD service: ${e.message}")
            }
        }
        multicastLock?.let {
            if (it.isHeld) {
                it.release()
                Logger.i("CoreInitializer: MulticastLock released")
            }
        }
        isInitialized = false
    }

    @Synchronized
    fun acquireMulticastLock(context: Context) {
        if (multicastLock == null) {
            val wifi = context.applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
            multicastLock = wifi.createMulticastLock("cdus_multicast_lock").apply {
                setReferenceCounted(false)
            }
        }
        multicastLock?.let {
            if (!it.isHeld) {
                it.acquire()
                Logger.i("CoreInitializer: MulticastLock acquired")
            }
        }
    }

    @Synchronized
    fun releaseMulticastLock() {
        multicastLock?.let {
            if (it.isHeld) {
                it.release()
                Logger.i("CoreInitializer: MulticastLock released")
            }
        }
    }
}
