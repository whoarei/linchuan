package io.github.whoarei.linchuan

import android.content.Context
import android.net.wifi.WifiManager
import android.os.Bundle
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  private var multicastLock: WifiManager.MulticastLock? = null

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
  }

  override fun onStart() {
    super.onStart()
    val wifi = applicationContext.getSystemService(Context.WIFI_SERVICE) as? WifiManager
    multicastLock = wifi?.createMulticastLock("linchuan-discovery")?.apply {
      setReferenceCounted(false)
      acquire()
    }
  }

  override fun onStop() {
    multicastLock?.let { if (it.isHeld) it.release() }
    multicastLock = null
    super.onStop()
  }
}
