<script setup lang="ts">
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { MonitorUp } from "lucide-vue-next";
import { computed, onMounted, onUnmounted, ref, shallowRef } from "vue";
import { api, ApiError } from "./api";
import type { Device } from "./types";
import { controlChannel, useScreenControl, type CaptureStatus, type NativeControlEvent } from "./useScreenControl";

interface Session {
  id: string;
  peer_id: string;
  peer_name: string;
  role: "sharer" | "viewer";
  accepted: boolean;
  offer: string | null;
  answer: string | null;
}
interface Call {
  id?: string;
  rtc?: RTCPeerConnection;
  capture?: MediaStream;
  answerApplied?: boolean;
  deadline?: number;
  androidToken?: string;
  linuxToken?: string;
  frameTimer?: number;
  browserTab?: string;
  heartbeat?: number;
}

const props = defineProps<{ browser: boolean; android: boolean; linux: boolean }>();
const emit = defineEmits<{ error: [message: string]; ended: [message: string] }>();
const session = shallowRef<Session | null>(null);
const remoteStream = shallowRef<MediaStream | null>(null);
const remoteFrame = ref("");
const peerName = ref("");
const role = ref<Session["role"]>();
const phase = ref<"" | "picking" | "inviting" | "waiting" | "connecting" | "connected">("");
const x11 = ref(false);
const canBrowserCapture = !!navigator.mediaDevices?.getDisplayMedia && !!window.RTCPeerConnection;
const canCapture = computed(() => !props.browser && (props.android || x11.value || canBrowserCapture));
const choosingSource = shallowRef<Device>();
const singleApp = ref(false);
const insets = ref({ top: 0, bottom: 0, left: 0, right: 0 });
const panelInsets = computed(() => ({
  "--assist-safe-top": `${insets.value.top}px`,
  "--assist-safe-bottom": `${insets.value.bottom}px`,
  "--assist-safe-left": `${insets.value.left}px`,
  "--assist-safe-right": `${insets.value.right}px`,
}));
let call: Call | undefined;
let poll: number | undefined;
let unlisten: UnlistenFn | undefined;
let unlistenControl: UnlistenFn | undefined;
let refreshing = false;
let disposed = false;
let closingId: string | undefined;
const control = useScreenControl(() => call?.id, (message) => emit("ended", message));
const { state: controlState, needsAccessibility, entry } = control;

const browserRoute = "/api/browser/assistance";
const browserControl = window.isSecureContext && window.location.protocol === "https:";

function tellPeerEnded(id: string, tab?: string) {
  const request = props.browser
    ? api(`${browserRoute}/${id}`, { action: "end", tab }, { keepalive: true })
    : invoke("assistance_end", { id });
  void request.catch(() => {});
}

function stop(announce = true) {
  const id = call?.id ?? session.value?.id;
  const previous = call;
  control.reset();
  choosingSource.value = undefined;
  call = undefined;
  window.clearTimeout(previous?.deadline);
  window.clearTimeout(previous?.frameTimer);
  window.clearInterval(previous?.heartbeat);
  if (previous?.androidToken) {
    void invoke("assistance_android_stop", { token: previous.androidToken }).catch(() => {});
  }
  if (previous?.linuxToken) {
    void invoke("assistance_linux_stop", { token: previous.linuxToken }).catch(() => {});
  }
  previous?.capture?.getTracks().forEach((track) => track.stop());
  previous?.rtc?.close();
  remoteStream.value = null;
  if (remoteFrame.value) URL.revokeObjectURL(remoteFrame.value);
  remoteFrame.value = "";
  phase.value = "";
  role.value = undefined;
  session.value = null;
  if (id && announce) {
    closingId = id;
    tellPeerEnded(id, previous?.browserTab);
  }
}

function report(error: unknown) {
  emit("error", typeof error === "string" ? error : error instanceof Error ? error.message : "无法建立屏幕共享");
}

function startDeadline(current: Call) {
  window.clearTimeout(current.deadline);
  current.deadline = window.setTimeout(() => {
    if (call !== current) return;
    stop();
    report("屏幕共享连接超时，已结束会话");
  }, 15_000);
}

function connect(current: Call) {
  const rtc = new RTCPeerConnection({ iceServers: [], bundlePolicy: "max-bundle" });
  current.rtc = rtc;
  rtc.onconnectionstatechange = () => {
    if (call !== current) return;
    switch (rtc.connectionState) {
      case "connected":
        window.clearTimeout(current.deadline);
        phase.value = "connected";
        break;
      case "disconnected":
        phase.value = "connecting";
        startDeadline(current);
        break;
      case "failed":
      case "closed":
        stop();
        report("屏幕共享连接已中断");
        break;
    }
  };
  return rtc;
}

async function gatheredSdp(rtc: RTCPeerConnection, description: RTCSessionDescriptionInit) {
  await rtc.setLocalDescription(description);
  if (rtc.iceGatheringState !== "complete") {
    await new Promise<void>((resolve) => {
      const timeout = window.setTimeout(done, 4_000);
      function done() {
        window.clearTimeout(timeout);
        rtc.removeEventListener("icegatheringstatechange", check);
        resolve();
      }
      function check() {
        if (rtc.iceGatheringState === "complete" || rtc.signalingState === "closed") done();
      }
      rtc.addEventListener("icegatheringstatechange", check);
      check();
    });
  }
  const sdp = rtc.localDescription?.sdp;
  if (!sdp?.includes("\r\na=candidate:")) throw new Error("找不到可用的局域网连接地址");
  return sdp;
}

function start(peer: Device) {
  if ((props.android && singleApp.value || x11.value) && !call && !session.value) {
    choosingSource.value = peer;
  } else void capture(peer, true);
}

function allowsControl(peer: Device) {
  return peer.kind === "app" || peer.capabilities.includes("screen-control-web-v1");
}

async function capture(peer: Device, entireDisplay: boolean) {
  if (!canCapture.value || call || session.value) return;
  choosingSource.value = undefined;
  const current: Call = {};
  call = current;
  phase.value = "picking";
  peerName.value = peer.name;
  role.value = "sharer";
  try {
    if (props.android) {
      const token = crypto.randomUUID();
      current.androidToken = token;
      const id = await invoke<string>("assistance_android_start", { peerId: peer.id, token, entireDisplay });
      if (call !== current) tellPeerEnded(id);
      else {
        current.id = id;
        phase.value = "waiting";
      }
      return;
    }
    if (props.linux && x11.value && entireDisplay) {
      const token = crypto.randomUUID();
      current.linuxToken = token;
      control.bindNative(token, true);
      const id = await invoke<string>("assistance_linux_start", { token, peerId: peer.id });
      if (call !== current) tellPeerEnded(id);
      else { current.id = id; phase.value = "waiting"; }
      return;
    }
    const capture = await navigator.mediaDevices.getDisplayMedia({
      audio: false,
      video: { width: { max: 1920 }, height: { max: 1080 }, frameRate: { max: 15 } },
    });
    if (call !== current) {
      capture.getTracks().forEach((track) => track.stop());
      return;
    }
    current.capture = capture;
    const track = capture.getVideoTracks()[0];
    if (!track || track.readyState !== "live") throw new Error("所选画面已不可用");
    track.contentHint = "detail";
    track.addEventListener("ended", () => { if (call === current) stop(); }, { once: true });
    phase.value = "inviting";
    const rtc = connect(current);
    rtc.addTransceiver(track, { direction: "sendonly" });
    if (allowsControl(peer) && track.getSettings().displaySurface === "monitor" && await invoke<boolean>("assistance_input_available")) {
      if (call !== current) return;
      control.bind(rtc.createDataChannel(controlChannel), true);
    }
    const sdp = await gatheredSdp(rtc, await rtc.createOffer());
    if (call !== current) return;
    const id = await invoke<string>("assistance_invite", { peerId: peer.id, sdp });
    if (call !== current) {
      tellPeerEnded(id);
      return;
    }
    current.id = id;
    phase.value = "waiting";
  } catch (error) {
    if (call !== current) return;
    stop();
    if (error instanceof DOMException && error.name === "NotAllowedError") {
      report("屏幕选择已取消，或系统未授予共享权限");
    } else if (!(error instanceof DOMException && error.name === "AbortError")) {
      report(error);
    }
  }
}

async function nextLinuxFrame(current: Call, token: string) {
  if (call !== current) return;
  try {
    const bytes = await invoke<ArrayBuffer>("assistance_linux_frame", { token });
    if (call !== current) return;
    if (bytes.byteLength) {
      const previous = remoteFrame.value;
      remoteFrame.value = URL.createObjectURL(new Blob([bytes], { type: "image/jpeg" }));
      if (previous) URL.revokeObjectURL(previous);
    }
    current.frameTimer = window.setTimeout(() => void nextLinuxFrame(current, token), 65);
  } catch (error) {
    if (call === current) { stop(); report(error); }
  }
}

async function join(invite: Session) {
  if (call || !invite.offer) return;
  const current: Call = { id: invite.id };
  call = current;
  role.value = "viewer";
  peerName.value = invite.peer_name;
  phase.value = "connecting";
  try {
    if (props.linux) {
      const token = crypto.randomUUID();
      current.linuxToken = token;
      control.bindNative(token, false);
      await invoke("assistance_linux_join", { token, id: invite.id, sdp: invite.offer });
      if (call === current) void nextLinuxFrame(current, token);
      return;
    }
    const rtc = connect(current);
    rtc.ondatachannel = ({ channel }) => {
      if (call !== current || (props.browser && !browserControl)) channel.close();
      else control.bind(channel, false);
    };
    rtc.ontrack = ({ track }) => {
      if (call === current && track.kind === "video") remoteStream.value = new MediaStream([track]);
    };
    await rtc.setRemoteDescription({ type: "offer", sdp: invite.offer });
    const transceivers = rtc.getTransceivers();
    if (transceivers.length !== 1 || transceivers[0].receiver.track.kind !== "video") {
      throw new Error("屏幕共享只接收一个画面来源");
    }
    transceivers[0].direction = "recvonly";
    const sdp = await gatheredSdp(rtc, await rtc.createAnswer());
    if (call !== current) return;
    startDeadline(current);
    if (props.browser) {
      const tab = Array.from(crypto.getRandomValues(new Uint8Array(16)),
        (byte) => byte.toString(16).padStart(2, "0")).join("");
      current.browserTab = tab;
      await api(`${browserRoute}/${invite.id}`, { action: "join", sdp, tab });
      if (call === current) {
        current.heartbeat = window.setInterval(() => {
          void api(`${browserRoute}/${invite.id}`, { action: "heartbeat", tab }).catch(refresh);
        }, 12_000);
      }
    } else {
      await invoke("assistance_join", { id: invite.id, sdp });
    }
  } catch (error) {
    if (call !== current) return;
    stop();
    report(error);
  }
}

async function refresh() {
  if (refreshing || disposed) return;
  refreshing = true;
  const current = call;
  const id = current?.id;
  const previous = session.value;
  try {
    const next = props.browser
      ? await api<Session | null>(browserRoute)
      : await invoke<Session | null>("assistance_session");
    if (disposed || call !== current || current?.id !== id || session.value !== previous) return;
    if (next && next.id === closingId) return;
    closingId = undefined;
    if (!current && next && (next.accepted || next.role === "sharer")) {
      if (props.browser) session.value = null;
      else tellPeerEnded(next.id);
      return;
    }
    session.value = next;
    if (id && next?.id !== id) {
      const message = phase.value === "waiting" ? "屏幕共享邀请已结束" : "屏幕共享会话已结束";
      stop(false);
      emit("ended", message);
      return;
    }
    if (current?.rtc && id && next?.answer && !current.answerApplied) {
      current.answerApplied = true;
      phase.value = "connecting";
      startDeadline(current);
      await current.rtc.setRemoteDescription({ type: "answer", sdp: next.answer });
    }
    if (current?.androidToken && id && next?.accepted) {
      const status = await invoke<CaptureStatus>("assistance_android_status", { token: current.androidToken });
      if (call === current) {
        phase.value = status.state === "connected" ? "connected" : "connecting";
        control.androidState(current.androidToken, status);
      }
    }
    if (current?.linuxToken && id && next?.accepted) {
      const status = await invoke<string>("assistance_linux_status", { token: current.linuxToken });
      if (call === current) phase.value = status === "connected" ? "connected" : "connecting";
    }
  } catch (error) {
    if (call === current && (current?.answerApplied || (props.browser && error instanceof ApiError && error.status === 401))) {
      stop();
      report(error);
    }
  } finally {
    refreshing = false;
  }
}

async function refreshAndroidCapabilities() {
  const capabilities = await invoke<typeof insets.value & { single_app: boolean }>("assistance_android_capabilities").catch(() => undefined);
  if (!disposed && capabilities) {
    singleApp.value = capabilities.single_app;
    const { top, bottom, left, right } = capabilities;
    insets.value = { top, bottom, left, right };
  }
}

onMounted(async () => {
  window.addEventListener("pagehide", onPageHide);
  if (props.android) {
    window.addEventListener("resize", refreshAndroidCapabilities);
    void refreshAndroidCapabilities();
  }
  if (props.linux) {
    void invoke<boolean>("assistance_input_available").then((available) => { x11.value = available; }).catch(() => {});
    unlistenControl = await listen<NativeControlEvent>("linchuan-linux-control", ({ payload }) => control.nativeEvent(payload)).catch(() => undefined);
  }
  if (!props.browser) unlisten = await listen("linchuan-assistance-stop", () => stop()).catch(() => undefined);
  if (disposed) {
    unlisten?.();
    unlistenControl?.();
    return;
  }
  void refresh();
  poll = window.setInterval(refresh, props.browser ? 3_000 : 700);
});
function onPageHide() { stop(!props.browser || !!call); }
onUnmounted(() => {
  disposed = true;
  window.removeEventListener("pagehide", onPageHide);
  window.removeEventListener("resize", refreshAndroidCapabilities);
  window.clearInterval(poll);
  unlisten?.();
  unlistenControl?.();
  onPageHide();
});

defineExpose({ start, canCapture, refresh });
</script>

<template>
  <section v-if="role === 'viewer'" class="screen-viewer" :style="panelInsets" aria-label="共享画面">
    <header>
      <div class="screen-copy" role="status">
        <strong v-if="controlState === 'granted'">正在控制 {{ peerName }}</strong>
        <strong v-else>{{ phase === "connected" ? `正在观看 ${peerName} 的屏幕` : `正在连接 ${peerName}` }}</strong>
        <small v-if="controlState === 'requested'">等待对方授权</small>
        <small v-else-if="browser && !browserControl && phase === 'connected'">通过 HTTPS 配对后可请求控制</small>
      </div>
      <div class="screen-actions">
        <button v-if="controlState === 'available'" type="button" class="primary" @click="control.request">请求控制</button>
        <button v-if="controlState === 'requested'" type="button" class="secondary" @click="control.cancel">取消请求</button>
        <button v-if="controlState === 'granted'" type="button" class="secondary" @click="control.cancel">交还控制</button>
        <button type="button" class="secondary" @click="stop()">离开会话</button>
      </div>
    </header>
    <video
      v-if="remoteStream"
      :srcObject.prop="remoteStream"
      :tabindex="controlState === 'granted' ? 0 : -1"
      :class="{ controllable: controlState === 'granted' }"
      autoplay playsinline muted aria-label="对方的共享画面"
      @pointerdown="control.down" @pointerup="control.up" @pointercancel="control.clearPointer"
      @contextmenu.prevent @wheel="control.wheel" @keydown="control.key"
    />
    <img
      v-else-if="remoteFrame"
      :src="remoteFrame"
      :tabindex="controlState === 'granted' ? 0 : -1"
      :class="{ controllable: controlState === 'granted' }"
      draggable="false" alt="对方的共享画面"
      @pointerdown="control.down" @pointerup="control.up" @pointercancel="control.clearPointer"
      @contextmenu.prevent @wheel="control.wheel" @keydown="control.key"
    />
    <p v-else role="status">正在等待对方的画面…</p>
    <form v-if="controlState === 'granted'" class="screen-text" @submit.prevent="control.submit">
      <input v-model="entry" aria-label="输入要发送给对方的文字" maxlength="400" placeholder="输入文字" />
      <button class="primary" :disabled="!entry">输入到对方设备</button>
    </form>
  </section>
  <section
    v-else-if="phase || session?.role === 'viewer'"
    class="screen-banner"
    :class="{ 'control-attention': controlState === 'requested' || controlState === 'granted' }"
    :style="panelInsets"
    aria-label="远程协助"
  >
    <MonitorUp :size="19" />
    <div v-if="phase" class="screen-copy" role="status">
      <strong v-if="phase === 'picking'">选择要分享的画面</strong>
      <strong v-else-if="phase === 'inviting'">正在邀请 {{ peerName }}</strong>
      <strong v-else-if="phase === 'waiting'">等待 {{ peerName }} 加入观看</strong>
      <strong v-else-if="phase === 'connecting'">正在连接 {{ peerName }}</strong>
      <template v-else-if="controlState === 'requested'">
        <strong>{{ peerName }} 请求控制</strong>
        <small v-if="needsAccessibility">需要先在系统设置启用邻传的协助触控权限。</small>
        <small v-else>允许后，对方可以操作此设备。</small>
      </template>
      <strong v-else-if="controlState === 'granted'">{{ peerName }} 正在控制你的设备</strong>
      <strong v-else>{{ peerName }} 正在观看你的屏幕</strong>
    </div>
    <div v-else-if="session" class="screen-copy" role="status">
      <strong>{{ session.peer_name }} 邀请你观看屏幕</strong>
      <small>加入即可观看对方选择的画面。</small>
    </div>
    <div class="screen-actions">
      <template v-if="controlState === 'requested'">
        <button type="button" class="secondary" @click="control.decide('refuse')">拒绝控制</button>
        <button v-if="needsAccessibility" type="button" class="primary" @click="control.decide('settings')">启用触控权限</button>
        <button v-else type="button" class="primary" @click="control.decide('grant')">允许控制</button>
      </template>
      <button v-if="controlState === 'granted'" type="button" class="primary" @click="control.decide('revoke')">收回控制</button>
      <button v-if="phase" type="button" class="secondary" @click="stop()">停止分享</button>
      <template v-else-if="session">
        <button type="button" class="secondary" @click="stop()">拒绝</button>
        <button type="button" class="primary" @click="join(session)">加入观看</button>
      </template>
    </div>
  </section>
  <section v-else-if="choosingSource" class="screen-banner" :style="panelInsets" aria-label="选择共享画面">
    <MonitorUp :size="19" />
    <div class="screen-copy"><strong>选择共享画面</strong><small v-if="allowsControl(choosingSource)">分享整个屏幕时，对方也可请求控制。</small></div>
    <div class="screen-actions">
      <button type="button" class="secondary" @click="choosingSource = undefined">取消</button>
      <button v-if="android" type="button" class="secondary" @click="capture(choosingSource, false)">选择应用</button>
      <button type="button" class="primary" @click="capture(choosingSource, true)">整个屏幕</button>
    </div>
  </section>
</template>

<style scoped>
.screen-banner,
.screen-viewer {
  position: fixed;
  z-index: 25;
  box-shadow: 0 10px 30px #23325328;
}
.screen-banner {
  top: calc(var(--titlebar-height) + max(var(--assist-safe-top), env(safe-area-inset-top, 0px)) + 10px);
  right: calc(max(var(--assist-safe-right), env(safe-area-inset-right, 0px)) + 12px);
  left: calc(max(var(--assist-safe-left), env(safe-area-inset-left, 0px)) + 12px);
  width: max-content;
  max-width: calc(100% - 24px);
  margin-inline: auto;
  padding: 10px 12px;
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 10px;
  border: 1px solid #ccdaf9;
  border-radius: 10px;
  background: #eef3fd;
  color: #315495;
}
.screen-copy { min-width: 0; overflow-wrap: anywhere; }
.screen-copy small { display: block; }
.screen-actions { display: flex; flex-wrap: wrap; gap: 6px; }
.screen-actions button { min-height: 35px; font-size: 12px; }
.screen-banner.control-attention { border-color: #e8d4a9; background: #fff8e8; color: #80521e; }
.screen-viewer {
  inset: calc(var(--titlebar-height) + max(var(--assist-safe-top), env(safe-area-inset-top, 0px)) + 12px)
    calc(max(var(--assist-safe-right), env(safe-area-inset-right, 0px)) + 12px)
    calc(max(var(--assist-safe-bottom), env(safe-area-inset-bottom, 0px)) + 12px)
    calc(max(var(--assist-safe-left), env(safe-area-inset-left, 0px)) + 12px);
  display: grid;
  grid-template-rows: auto minmax(0, 1fr);
  border: 1px solid #c8d4eb;
  border-radius: 12px;
  overflow: hidden;
  background: #142038;
  color: #f5f7fc;
}
.screen-viewer header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
  padding: 10px 14px;
  background: #f7f9fd;
  color: #253047;
}
.screen-viewer :is(video, img) { display: block; width: 100%; height: 100%; object-fit: contain; }
.screen-viewer .controllable { touch-action: none; cursor: crosshair; }
.screen-viewer :is(video, img):focus-visible { outline: 2px solid #90acff; outline-offset: -2px; }
.screen-viewer p { place-self: center; }
.screen-viewer:has(.screen-text) { grid-template-rows: auto minmax(0, 1fr) auto; }
.screen-text { display: flex; gap: 8px; padding: 10px; background: #f7f9fd; }
.screen-text input { flex: 1; min-width: 0; padding: 7px 9px; border: 1px solid #d5deef; border-radius: 7px; color: #253047; }
</style>
