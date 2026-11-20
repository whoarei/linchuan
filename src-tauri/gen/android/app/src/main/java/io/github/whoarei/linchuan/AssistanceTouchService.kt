package io.github.whoarei.linchuan

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.GestureDescription
import android.accessibilityservice.InputMethod
import android.graphics.Path
import android.os.Build
import android.os.Bundle
import android.text.InputType
import android.view.KeyEvent
import android.view.WindowManager
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import org.json.JSONObject

class AssistanceTouchService : AccessibilityService() {
  companion object {
    var instance: AssistanceTouchService? = null
      private set
  }

  private var touching = false

  override fun onServiceConnected() { instance = this }
  override fun onCreateInputMethod(): InputMethod = InputMethod(this)
  override fun onAccessibilityEvent(event: AccessibilityEvent?) {}
  override fun onInterrupt() { ScreenShareService.active?.control("revoke") }
  override fun onDestroy() {
    instance = null
    onInterrupt()
    super.onDestroy()
  }

  fun applyInput(input: JSONObject) {
    when (input.optString("kind")) {
      "click", "drag", "scroll" -> touch(input)
      "text" -> editText(input.optString("text"), false)
      "key" -> editKey(input.optString("key"))
    }
  }

  private fun touch(input: JSONObject) {
    if (touching) return
    val (width, height) = if (Build.VERSION.SDK_INT >= 30) {
      val bounds = getSystemService(WindowManager::class.java).maximumWindowMetrics.bounds
      Pair(bounds.width(), bounds.height())
    } else Pair(resources.displayMetrics.widthPixels, resources.displayMetrics.heightPixels)
    val x = input.optDouble("x")
    val y = input.optDouble("y")
    if (x !in 0.0..1.0 || y !in 0.0..1.0) return
    val path = Path().apply { moveTo((x * (width - 1)).toFloat(), (y * (height - 1)).toFloat()) }
    val duration = when (input.optString("kind")) {
      "drag" -> {
        val endX = input.optDouble("to_x")
        val endY = input.optDouble("to_y")
        if (endX !in 0.0..1.0 || endY !in 0.0..1.0) return
        path.lineTo((endX * (width - 1)).toFloat(), (endY * (height - 1)).toFloat())
        170L
      }
      "scroll" -> {
        val lines = input.optInt("lines").coerceIn(-6, 6)
        path.lineTo((x * (width - 1)).toFloat(), ((y - lines * 0.12).coerceIn(0.0, 1.0) * (height - 1)).toFloat())
        160L
      }
      else -> if (input.optInt("button") == 2) 550L else 45L
    }
    val gesture = GestureDescription.Builder().addStroke(GestureDescription.StrokeDescription(path, 0, duration)).build()
    touching = dispatchGesture(gesture, object : GestureResultCallback() {
      override fun onCompleted(gestureDescription: GestureDescription?) { touching = false }
      override fun onCancelled(gestureDescription: GestureDescription?) { touching = false }
    }, null)
  }

  @Suppress("DEPRECATION")
  private fun editText(entered: String, deleting: Boolean) {
    if (entered.length > 400 || entered.any { it.isISOControl() }) return
    if (!deleting && Build.VERSION.SDK_INT >= 33) {
      val connection = inputMethod?.currentInputConnection
      if (connection != null) {
        if (!isPasswordEditor()) connection.commitText(entered, 1, null)
        return
      }
    }
    val field = findFocus(AccessibilityNodeInfo.FOCUS_INPUT) ?: return
    try {
      if (!field.isEditable || field.isPassword) return
      val value = field.text?.toString().orEmpty()
      val caret = field.textSelectionStart.takeIf { it in 0..value.length } ?: value.length
      val end = field.textSelectionEnd.takeIf { it in 0..value.length } ?: caret
      var start = minOf(caret, end)
      val finish = maxOf(caret, end)
      if (deleting && start == finish && start > 0) start = value.offsetByCodePoints(start, -1)
      val changed = value.replaceRange(start, finish, entered)
      if (field.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, Bundle().apply {
        putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, changed)
      })) {
        field.performAction(AccessibilityNodeInfo.ACTION_SET_SELECTION, Bundle().apply {
          putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, start + entered.length)
          putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, start + entered.length)
        })
      }
    } finally {
      field.recycle()
    }
  }

  private fun editKey(key: String) {
    if (Build.VERSION.SDK_INT >= 33) {
      val connection = inputMethod?.currentInputConnection
      if (connection != null) {
        if (isPasswordEditor()) return
        val code = when (key) {
          "Enter" -> KeyEvent.KEYCODE_ENTER
          "Backspace" -> KeyEvent.KEYCODE_DEL
          "Delete" -> KeyEvent.KEYCODE_FORWARD_DEL
          "ArrowUp" -> KeyEvent.KEYCODE_DPAD_UP
          "ArrowDown" -> KeyEvent.KEYCODE_DPAD_DOWN
          "ArrowLeft" -> KeyEvent.KEYCODE_DPAD_LEFT
          "ArrowRight" -> KeyEvent.KEYCODE_DPAD_RIGHT
          else -> return
        }
        connection.sendKeyEvent(KeyEvent(KeyEvent.ACTION_DOWN, code))
        connection.sendKeyEvent(KeyEvent(KeyEvent.ACTION_UP, code))
        return
      }
    }
    if (key == "Backspace") editText("", true)
  }

  @androidx.annotation.RequiresApi(33)
  private fun isPasswordEditor(): Boolean {
    val type = inputMethod?.currentInputEditorInfo?.inputType ?: return true
    val variation = type and InputType.TYPE_MASK_VARIATION
    return when (type and InputType.TYPE_MASK_CLASS) {
      InputType.TYPE_CLASS_TEXT -> variation == InputType.TYPE_TEXT_VARIATION_PASSWORD ||
        variation == InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD || variation == InputType.TYPE_TEXT_VARIATION_WEB_PASSWORD
      InputType.TYPE_CLASS_NUMBER -> variation == InputType.TYPE_NUMBER_VARIATION_PASSWORD
      else -> false
    }
  }
}
