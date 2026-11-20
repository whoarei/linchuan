package io.github.whoarei.linchuan

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.media.projection.MediaProjection
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.view.WindowManager
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import org.webrtc.DataChannel
import org.webrtc.DefaultVideoDecoderFactory
import org.webrtc.DefaultVideoEncoderFactory
import org.webrtc.EglBase
import org.webrtc.IceCandidate
import org.webrtc.MediaConstraints
import org.webrtc.MediaStream
import org.webrtc.PeerConnection
import org.webrtc.PeerConnectionFactory
import org.webrtc.RtpReceiver
import org.webrtc.RtpTransceiver
import org.webrtc.ScreenCapturerAndroid
import org.webrtc.SdpObserver
import org.webrtc.SessionDescription
import org.webrtc.SurfaceTextureHelper
import org.webrtc.VideoSource
import org.webrtc.VideoTrack
import org.json.JSONObject
import java.nio.ByteBuffer
import java.util.UUID
import kotlin.math.roundToInt

class ScreenShareService : Service() {
  data class Launch(val token: String, val invoke: Invoke, val entireDisplay: Boolean, val allowControl: Boolean)

  companion object {
    const val EXTRA_TOKEN = "capture_token"
    const val EXTRA_CONSENT = "capture_consent"
    private const val STOP = "io.github.whoarei.linchuan.STOP_SCREEN_SHARE"
    private const val REVOKE = "io.github.whoarei.linchuan.REVOKE_SCREEN_CONTROL"
    private const val CHANNEL = "screen-sharing"
    private const val NOTIFICATION = 6203
    var pending: Launch? = null
    var active: ScreenShareService? = null
    private var initialized = false
  }

  var token = ""
    private set
  var connectionState = "waiting"
    private set
  var controlState = "unavailable"
    private set
  private val main = Handler(Looper.getMainLooper())
  private var peer: PeerConnection? = null
  private var capturer: ScreenCapturerAndroid? = null
  private var source: VideoSource? = null
  private var track: VideoTrack? = null
  private var factory: PeerConnectionFactory? = null
  private var helper: SurfaceTextureHelper? = null
  private var egl: EglBase? = null
  private var offerRequest: Invoke? = null
  private var stopping = false
  private var canControl = false
  private var channel: DataChannel? = null
  private var lease: String? = null

  override fun onBind(intent: Intent?): IBinder? = null

  @Suppress("DEPRECATION")
  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    val requestedToken = intent?.getStringExtra(EXTRA_TOKEN)
    if (intent?.action == STOP) {
      if (requestedToken == token) stopSharing()
      return START_NOT_STICKY
    }
    if (intent?.action == REVOKE) {
      if (requestedToken == token) control("revoke")
      return START_NOT_STICKY
    }
    val launch = pending?.takeIf { requestedToken == it.token }
    val consent = if (Build.VERSION.SDK_INT >= 33)
      intent?.getParcelableExtra(EXTRA_CONSENT, Intent::class.java)
    else intent?.getParcelableExtra<Intent>(EXTRA_CONSENT)
    if (launch == null || consent == null || active != null) {
      if (active == null) stopSelf()
      return START_NOT_STICKY
    }
    token = launch.token
    canControl = launch.entireDisplay && launch.allowControl
    pending = null
    offerRequest = launch.invoke
    active = this
    try {
      showNotification()
      setupCapture(consent)
    } catch (error: Exception) {
      stopSharing(error.message ?: "系统未能开始屏幕共享")
    }
    return START_NOT_STICKY
  }

  private fun showNotification() {
    val manager = getSystemService(NotificationManager::class.java)
    if (Build.VERSION.SDK_INT >= 26) {
      manager.createNotificationChannel(NotificationChannel(CHANNEL, "屏幕共享", NotificationManager.IMPORTANCE_LOW))
    }
    val stop = PendingIntent.getService(this, NOTIFICATION,
      Intent(this, ScreenShareService::class.java).setAction(STOP).putExtra(EXTRA_TOKEN, token),
      PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    val open = PendingIntent.getActivity(this, NOTIFICATION, Intent(this, MainActivity::class.java),
      PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    val notification = NotificationCompat.Builder(this, CHANNEL)
      .setSmallIcon(android.R.drawable.ic_menu_camera)
      .setContentTitle("邻传正在共享屏幕")
      .setContentText(when (controlState) {
        "granted" -> "协助方正在操作此设备。"
        "requested" -> "协助方请求控制，轻触返回确认。"
        else -> "轻触返回会话，或停止共享。"
      })
      .setContentIntent(open)
      .addAction(android.R.drawable.ic_menu_close_clear_cancel, "停止共享", stop)
      .setOngoing(true)
    if (controlState == "granted") {
      val revoke = PendingIntent.getService(this, NOTIFICATION + 1,
        Intent(this, ScreenShareService::class.java).setAction(REVOKE).putExtra(EXTRA_TOKEN, token),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
      notification.addAction(android.R.drawable.ic_menu_revert, "收回控制", revoke)
    }
    ServiceCompat.startForeground(this, NOTIFICATION, notification.build(),
      if (Build.VERSION.SDK_INT >= 29) ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION else 0)
  }

  private fun captureSize(width: Int, height: Int): Pair<Int, Int> {
    val scale = minOf(1.0, 1280.0 / maxOf(width, height), 800.0 / minOf(width, height))
    return Pair((width * scale).roundToInt().coerceAtLeast(2), (height * scale).roundToInt().coerceAtLeast(2))
  }

  private fun setupCapture(consent: Intent) {
    if (!initialized) {
      PeerConnectionFactory.initialize(PeerConnectionFactory.InitializationOptions.builder(this).createInitializationOptions())
      initialized = true
    }
    val graphics = EglBase.create().also { egl = it }
    val rtcFactory = PeerConnectionFactory.builder()
      .setVideoEncoderFactory(DefaultVideoEncoderFactory(graphics.eglBaseContext, true, true))
      .setVideoDecoderFactory(DefaultVideoDecoderFactory(graphics.eglBaseContext))
      .createPeerConnectionFactory().also { factory = it }
    val video = rtcFactory.createVideoSource(true).also { source = it }
    val texture = SurfaceTextureHelper.create("linchuan-capture", graphics.eglBaseContext).also { helper = it }
    val projection = ScreenCapturerAndroid(consent, object : MediaProjection.Callback() {
      override fun onStop() { main.post { stopSharing() } }
      override fun onCapturedContentResize(width: Int, height: Int) {
        if (width > 0 && height > 0) main.post {
          if (canControl && Build.VERSION.SDK_INT >= 30) {
            val bounds = getSystemService(WindowManager::class.java).maximumWindowMetrics.bounds
            if (width != bounds.width() || height != bounds.height()) {
              control("revoke")
              canControl = false
              setControl("unavailable")
              channel?.close()
            }
          }
          val (w, h) = captureSize(width, height)
          capturer?.changeCaptureFormat(w, h, 15)
        }
      }
    }).also { capturer = it }
    projection.initialize(texture, this, video.capturerObserver)
    val (width, height) = if (Build.VERSION.SDK_INT >= 30) {
      val bounds = getSystemService(WindowManager::class.java).maximumWindowMetrics.bounds
      Pair(bounds.width(), bounds.height())
    } else Pair(resources.displayMetrics.widthPixels, resources.displayMetrics.heightPixels)
    val (w, h) = captureSize(width, height)
    projection.startCapture(w, h, 15)

    val config = PeerConnection.RTCConfiguration(emptyList()).apply {
      bundlePolicy = PeerConnection.BundlePolicy.MAXBUNDLE
    }
    val connection = rtcFactory.createPeerConnection(config, observer)
      ?: error("无法创建屏幕共享连接")
    peer = connection
    val streamTrack = rtcFactory.createVideoTrack("screen", video).also { track = it }
    connection.addTransceiver(streamTrack, RtpTransceiver.RtpTransceiverInit(RtpTransceiver.RtpTransceiverDirection.SEND_ONLY))
    if (canControl) bindControl(connection.createDataChannel("linchuan-control-v1", DataChannel.Init()))
    connection.createOffer(object : SdpResult() {
      override fun onCreateSuccess(sdp: SessionDescription) {
        main.post {
          if (!stopping) connection.setLocalDescription(object : SdpResult() {
            override fun onSetSuccess() { main.post { main.postDelayed({ finishOffer(required = true) }, 4000) } }
          }, sdp)
        }
      }
    }, MediaConstraints())
  }

  private fun finishOffer(required: Boolean = false) {
    val invoke = offerRequest ?: return
    val sdp = peer?.localDescription?.description
    if (sdp == null || !sdp.contains("\r\na=candidate:")) {
      if (required) stopSharing("找不到可用的局域网连接地址")
      return
    }
    offerRequest = null
    invoke.resolve(JSObject().put("sdp", sdp))
  }

  fun setAnswer(sdp: String, invoke: Invoke) {
    val connection = peer
    if (connection == null || stopping) {
      invoke.reject("屏幕共享已结束")
      return
    }
    connectionState = "connecting"
    connection.setRemoteDescription(object : SdpResult() {
      override fun onSetSuccess() { invoke.resolve(JSObject()) }
      override fun onSetFailure(error: String) { invoke.reject(error) }
    }, SessionDescription(SessionDescription.Type.ANSWER, sdp))
  }

  private fun setControl(state: String) {
    if (controlState == state) return
    controlState = state
    if (!stopping) showNotification()
  }

  private fun sendControl(type: String, lease: String? = null) {
    val transport = channel ?: return
    if (transport.state() != DataChannel.State.OPEN || transport.bufferedAmount() >= 8192) return
    val data = JSONObject().put("type", type)
    if (lease != null) data.put("lease", lease)
    transport.send(DataChannel.Buffer(ByteBuffer.wrap(data.toString().toByteArray(Charsets.UTF_8)), false))
  }

  fun control(action: String) {
    when (action) {
      "grant" -> {
        check(canControl && connectionState == "connected" && controlState == "requested") { "控制请求已失效" }
        check(AssistanceTouchService.instance != null) { "请先启用邻传的协助触控权限" }
        val key = UUID.randomUUID().toString()
        lease = key
        setControl("granted")
        sendControl("granted", key)
      }
      "refuse" -> if (controlState == "requested") {
        setControl("available")
        sendControl("refused")
      }
      "revoke" -> if (controlState == "granted") {
        lease = null
        setControl("available")
        sendControl("revoked")
      }
    }
  }

  private fun bindControl(transport: DataChannel) {
    channel = transport
    transport.registerObserver(object : DataChannel.Observer {
      override fun onBufferedAmountChange(previousAmount: Long) {}
      override fun onStateChange() {
        main.post {
          if (stopping) return@post
          if (canControl && transport.state() == DataChannel.State.OPEN) {
            setControl("available")
            sendControl("available")
          } else if (transport.state() == DataChannel.State.CLOSED) {
            lease = null
            setControl("unavailable")
          }
        }
      }
      override fun onMessage(buffer: DataChannel.Buffer) {
        val size = buffer.data.remaining()
        if (buffer.binary || size > 2200) return
        val bytes = ByteArray(size)
        buffer.data.get(bytes)
        main.post {
          if (!canControl || stopping || connectionState != "connected") return@post
          val message = try { JSONObject(bytes.toString(Charsets.UTF_8)) } catch (_: Exception) { return@post }
          when (message.optString("type")) {
            "request" -> if (controlState == "available") setControl("requested")
            "cancel" -> if (controlState == "requested") setControl("available")
            "release" -> control("revoke")
            "input" -> if (controlState == "granted" && lease != null && message.optString("lease") == lease) {
              val input = message.optJSONObject("input") ?: return@post
              val touch = AssistanceTouchService.instance ?: run { control("revoke"); return@post }
              touch.applyInput(input)
            }
          }
        }
      }
    })
  }

  private open inner class SdpResult : SdpObserver {
    override fun onCreateSuccess(sdp: SessionDescription) {}
    override fun onSetSuccess() {}
    override fun onCreateFailure(error: String) { main.post { stopSharing(error) } }
    override fun onSetFailure(error: String) { main.post { stopSharing(error) } }
  }

  private val observer = object : PeerConnection.Observer {
    override fun onSignalingChange(value: PeerConnection.SignalingState) {}
    override fun onIceConnectionChange(value: PeerConnection.IceConnectionState) {}
    override fun onIceConnectionReceivingChange(value: Boolean) {}
    override fun onIceGatheringChange(value: PeerConnection.IceGatheringState) {
      if (value == PeerConnection.IceGatheringState.COMPLETE) main.post { finishOffer() }
    }
    override fun onConnectionChange(value: PeerConnection.PeerConnectionState) {
      main.post {
        if (!stopping) when (value) {
          PeerConnection.PeerConnectionState.CONNECTED -> connectionState = "connected"
          PeerConnection.PeerConnectionState.DISCONNECTED -> {
            connectionState = "connecting"
            control("revoke")
          }
          PeerConnection.PeerConnectionState.FAILED, PeerConnection.PeerConnectionState.CLOSED -> stopSharing()
          else -> Unit
        }
      }
    }
    override fun onIceCandidate(candidate: IceCandidate) {}
    override fun onIceCandidatesRemoved(candidates: Array<IceCandidate>) {}
    override fun onAddStream(stream: MediaStream) {}
    override fun onRemoveStream(stream: MediaStream) {}
    override fun onDataChannel(channel: DataChannel) {}
    override fun onRenegotiationNeeded() {}
    override fun onAddTrack(receiver: RtpReceiver, streams: Array<MediaStream>) {}
  }

  fun stopSharing(message: String = "屏幕共享已结束") {
    if (stopping) return
    stopping = true
    lease = null
    active = null
    connectionState = "ended"
    offerRequest?.reject(message)
    offerRequest = null
    main.removeCallbacksAndMessages(null)
    try { capturer?.stopCapture() } catch (_: Exception) {}
    capturer?.dispose()
    channel?.unregisterObserver()
    channel?.dispose()
    peer?.dispose()
    track?.dispose()
    source?.dispose()
    helper?.dispose()
    factory?.dispose()
    egl?.release()
    ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
    stopSelf()
  }

  override fun onDestroy() {
    stopSharing()
    super.onDestroy()
  }
}
