package io.github.whoarei.linchuan

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.media.projection.MediaProjectionConfig
import android.media.projection.MediaProjectionManager
import android.os.Build
import android.provider.Settings
import androidx.activity.result.ActivityResult
import androidx.core.content.ContextCompat
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class ScreenShareArgs {
  var token = ""
  var sdp = ""
  var entireDisplay = true
  var allowControl = false
  var action = ""
}

@TauriPlugin(permissions = [Permission(alias = "share-notifications", strings = [Manifest.permission.POST_NOTIFICATIONS])])
class ScreenAssistancePlugin(private val activity: Activity) : Plugin(activity) {
  @Command
  fun startCapture(invoke: Invoke) = activity.runOnUiThread {
    val args = invoke.parseArgs(ScreenShareArgs::class.java)
    if (ScreenShareService.pending != null || ScreenShareService.active != null) {
      invoke.reject("请先结束当前屏幕共享")
      return@runOnUiThread
    }
    ScreenShareService.pending = ScreenShareService.Launch(args.token, invoke, args.entireDisplay, args.allowControl)
    val preferences = activity.getSharedPreferences("screen-assistance", Activity.MODE_PRIVATE)
    if (Build.VERSION.SDK_INT >= 33 && !preferences.getBoolean("notification_asked", false)
      && activity.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
      preferences.edit().putBoolean("notification_asked", true).apply()
      requestPermissionForAlias("share-notifications", invoke, "notificationResult")
    } else selectScreen(invoke)
  }

  @PermissionCallback
  fun notificationResult(invoke: Invoke) { selectScreen(invoke) }

  private fun selectScreen(invoke: Invoke) {
    val launch = ScreenShareService.pending?.takeIf { it.invoke === invoke } ?: return
    try {
      val manager = activity.getSystemService(MediaProjectionManager::class.java)
      val intent = if (Build.VERSION.SDK_INT >= 34 && launch.entireDisplay)
        manager.createScreenCaptureIntent(MediaProjectionConfig.createConfigForDefaultDisplay())
      else manager.createScreenCaptureIntent()
      startActivityForResult(invoke, intent, "captureResult")
    } catch (error: Exception) {
      ScreenShareService.pending = null
      invoke.reject(error.message ?: "无法打开系统共享选择器")
    }
  }

  @ActivityCallback
  fun captureResult(invoke: Invoke, result: ActivityResult) {
    val token = invoke.parseArgs(ScreenShareArgs::class.java).token
    if (ScreenShareService.pending?.token != token) return
    val consent = result.data
    if (result.resultCode != Activity.RESULT_OK || consent == null) {
      ScreenShareService.pending = null
      invoke.reject("已取消选择共享画面")
      return
    }
    try {
      val intent = Intent(activity, ScreenShareService::class.java)
        .putExtra(ScreenShareService.EXTRA_TOKEN, token)
        .putExtra(ScreenShareService.EXTRA_CONSENT, consent)
      ContextCompat.startForegroundService(activity, intent)
    } catch (error: Exception) {
      ScreenShareService.pending = null
      invoke.reject(error.message ?: "无法启动屏幕共享服务")
    }
  }

  @Command
  fun setAnswer(invoke: Invoke) = activity.runOnUiThread {
    val args = invoke.parseArgs(ScreenShareArgs::class.java)
    val share = ScreenShareService.active?.takeIf { it.token == args.token }
    if (share == null) invoke.reject("屏幕共享已结束")
    else share.setAnswer(args.sdp, invoke)
  }

  @Command
  fun captureState(invoke: Invoke) = activity.runOnUiThread {
    val token = invoke.parseArgs(ScreenShareArgs::class.java).token
    val share = ScreenShareService.active?.takeIf { it.token == token }
    invoke.resolve(JSObject().put("state", share?.connectionState ?: "ended")
      .put("control", share?.controlState ?: "unavailable")
      .put("needs_accessibility", AssistanceTouchService.instance == null))
  }

  @Command
  fun captureCapabilities(invoke: Invoke) = activity.runOnUiThread {
    val systemBars = ViewCompat.getRootWindowInsets(activity.window.decorView)?.getInsetsIgnoringVisibility(
      WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout()
    )
    val density = activity.resources.displayMetrics.density.toDouble()
    invoke.resolve(JSObject().put("single_app", Build.VERSION.SDK_INT >= 34)
      .put("top", (systemBars?.top ?: 0) / density)
      .put("bottom", (systemBars?.bottom ?: 0) / density)
      .put("left", (systemBars?.left ?: 0) / density)
      .put("right", (systemBars?.right ?: 0) / density))
  }

  @Command
  fun controlAction(invoke: Invoke) = activity.runOnUiThread {
    val args = invoke.parseArgs(ScreenShareArgs::class.java)
    val share = ScreenShareService.active?.takeIf { it.token == args.token }
    if (share == null) { invoke.reject("屏幕共享已结束"); return@runOnUiThread }
    try {
      if (args.action == "settings") activity.startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS))
      else share.control(args.action)
      invoke.resolve(JSObject())
    } catch (error: Exception) { invoke.reject(error.message ?: "无法更改控制权限") }
  }

  @Command
  fun stopCapture(invoke: Invoke) = activity.runOnUiThread {
    val token = invoke.parseArgs(ScreenShareArgs::class.java).token
    val pending = ScreenShareService.pending?.takeIf { it.token == token }
    if (pending != null) {
      ScreenShareService.pending = null
      pending.invoke.reject("屏幕共享已取消")
    }
    ScreenShareService.active?.takeIf { it.token == token }?.stopSharing()
    invoke.resolve(JSObject())
  }
}
