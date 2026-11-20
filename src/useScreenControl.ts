import { invoke } from "@tauri-apps/api/core";
import { ref } from "vue";

export const controlChannel = "linchuan-control-v1";
type State = "unavailable" | "available" | "requested" | "granted";
export interface CaptureStatus {
  state: string;
  control: State;
  needs_accessibility: boolean;
}
type Channel = Pick<RTCDataChannel, "label" | "readyState" | "bufferedAmount" | "send" | "close" | "onopen" | "onclose" | "onmessage">;
export interface NativeControlEvent {
  token: string;
  kind: "open" | "close" | "message";
  message: string | null;
}

export function useScreenControl(sessionId: () => string | undefined, notify: (message: string) => void) {
  const state = ref<State>("unavailable");
  const needsAccessibility = ref(false);
  const entry = ref("");
  let connection: { channel: Channel; sharer: boolean } | undefined;
  let native: { token: string; channel: Channel } | undefined;
  let lease: string | undefined;
  let sharingToken: string | undefined;
  let runningInput = false;
  let pointerStart: { x: number; y: number; button: number } | undefined;

  function send(message: Record<string, unknown>) {
    const channel = connection?.channel;
    if (channel?.readyState === "open" && channel.bufferedAmount < 8192) {
      channel.send(JSON.stringify(message));
    }
  }

  function release(id = sessionId()) {
    if (connection?.sharer && id && lease) {
      void invoke("assistance_input_grant", { id, lease: null }).catch(() => {});
    }
    lease = undefined;
    pointerStart = undefined;
    entry.value = "";
  }

  function reset() {
    release();
    native = undefined;
    connection = undefined;
    sharingToken = undefined;
    state.value = "unavailable";
    needsAccessibility.value = false;
  }

  function bind(channel: Channel, sharer: boolean) {
    if (channel.label !== controlChannel || connection) {
      channel.close();
      return;
    }
    const bound = { channel, sharer };
    connection = bound;
    channel.onopen = () => {
      if (connection !== bound || !sharer) return;
      state.value = "available";
      send({ type: "available" });
    };
    channel.onclose = () => { if (connection === bound) reset(); };
    channel.onmessage = async ({ data }) => {
      if (connection !== bound || typeof data !== "string" || data.length > 2200) return;
      try {
        const message = JSON.parse(data);
        if (!message || typeof message !== "object") return;
        if (sharer) {
          if (message.type === "request" && state.value === "available") state.value = "requested";
          if (message.type === "cancel" && state.value === "requested") state.value = "available";
          if (message.type === "release" && state.value === "granted") revoke();
          if (message.type === "input" && state.value === "granted" && lease && message.lease === lease && !runningInput) {
            const id = sessionId();
            if (!id) return;
            runningInput = true;
            try {
              await invoke("assistance_input_apply", { id, lease, input: message.input });
            } catch {
              if (connection === bound) revoke();
            } finally {
              runningInput = false;
            }
          }
        } else {
          if (message.type === "available" && state.value === "unavailable") state.value = "available";
          if (message.type === "granted" && state.value === "requested" && typeof message.lease === "string" && message.lease.length === 36) {
            state.value = "granted";
            lease = message.lease;
          }
          if (["refused", "revoked"].includes(message.type) && state.value !== "unavailable") {
            const previous = state.value;
            release();
            state.value = "available";
            if (previous !== "available") notify(message.type === "refused" ? "对方拒绝了控制请求" : "控制已结束，仍可继续观看");
          }
        }
      } catch { /* Ignore malformed messages. */ }
    };
  }

  function bindNative(token: string, sharer: boolean) {
    const channel = {
      label: controlChannel, readyState: "connecting", bufferedAmount: 0,
      send: (message: string) => { void invoke("assistance_linux_message", { token, message }).catch(() => {}); },
      close: () => {}, onopen: null, onclose: null, onmessage: null,
    } as Channel;
    native = { token, channel };
    bind(channel, sharer);
  }

  function nativeEvent(event: NativeControlEvent) {
    if (native?.token !== event.token) return;
    const channel = native.channel;
    if (event.kind === "open") {
      Object.assign(channel, { readyState: "open" });
      channel.onopen?.call(channel as RTCDataChannel, new Event("open"));
    } else if (event.kind === "close") {
      Object.assign(channel, { readyState: "closed" });
      channel.onclose?.call(channel as RTCDataChannel, new Event("close"));
    } else if (event.message !== null) {
      channel.onmessage?.call(channel as RTCDataChannel, new MessageEvent("message", { data: event.message }));
    }
  }

  function androidState(token: string, status: CaptureStatus) {
    sharingToken = token;
    state.value = status.control;
    needsAccessibility.value = status.needs_accessibility;
  }

  function request() {
    if (state.value !== "available") return;
    send({ type: "request" });
    state.value = "requested";
  }

  function cancel() {
    send({ type: state.value === "granted" ? "release" : "cancel" });
    release();
    state.value = "available";
  }

  async function decide(action: "grant" | "refuse" | "revoke" | "settings") {
    try {
      if (sharingToken) {
        await invoke("assistance_android_control", { token: sharingToken, action });
        return;
      }
      if (action === "revoke") { revoke(); return; }
      if (action === "refuse") { state.value = "available"; send({ type: "refused" }); return; }
      const id = sessionId();
      const bound = connection;
      if (action !== "grant" || !id || !bound?.sharer || state.value !== "requested") return;
      const candidate = crypto.randomUUID();
      await invoke("assistance_input_grant", { id, lease: candidate });
      if (connection !== bound || sessionId() !== id || state.value !== "requested") {
        await invoke("assistance_input_grant", { id, lease: null });
        return;
      }
      lease = candidate;
      state.value = "granted";
      send({ type: "granted", lease });
    } catch (error) {
      notify(typeof error === "string" ? error : "操作失败，请重试");
    }
  }

  function revoke() {
    release();
    state.value = "available";
    send({ type: "revoked" });
  }

  function input(value: Record<string, unknown>) {
    if (state.value === "granted" && !connection?.sharer && lease) send({ type: "input", lease, input: value });
  }

  function point(event: MouseEvent | WheelEvent) {
    const surface = event.currentTarget;
    const size = surface instanceof HTMLVideoElement ? [surface.videoWidth, surface.videoHeight]
      : surface instanceof HTMLImageElement ? [surface.naturalWidth, surface.naturalHeight] : undefined;
    if (!size?.[0] || !size[1] || !(surface instanceof HTMLElement)) return;
    const bounds = surface.getBoundingClientRect();
    const scale = Math.min(bounds.width / size[0], bounds.height / size[1]);
    const width = size[0] * scale;
    const height = size[1] * scale;
    if (width <= 0 || height <= 0) return;
    const x = (event.clientX - bounds.left - (bounds.width - width) / 2) / width;
    const y = (event.clientY - bounds.top - (bounds.height - height) / 2) / height;
    if (x >= 0 && x <= 1 && y >= 0 && y <= 1) return { x, y };
  }

  function down(event: PointerEvent) {
    if (state.value !== "granted") return;
    const location = point(event);
    const surface = event.currentTarget as HTMLElement;
    if (!location || ![0, 2].includes(event.button)) return;
    event.preventDefault();
    surface.focus();
    surface.setPointerCapture(event.pointerId);
    pointerStart = { ...location, button: event.button };
  }

  function up(event: PointerEvent) {
    const start = pointerStart;
    pointerStart = undefined;
    const end = point(event);
    if (!start || !end) return;
    if (start.button === 0 && Math.hypot(start.x - end.x, start.y - end.y) > 0.012) {
      input({ kind: "drag", x: start.x, y: start.y, to_x: end.x, to_y: end.y });
    } else input({ kind: "click", ...end, button: start.button });
  }

  function wheel(event: WheelEvent) {
    const location = point(event);
    if (state.value !== "granted" || !location || !event.deltaY) return;
    event.preventDefault();
    input({ kind: "scroll", ...location, lines: Math.sign(event.deltaY) * 2 });
  }

  function key(event: KeyboardEvent) {
    if (state.value !== "granted" || event.isComposing || event.ctrlKey || event.metaKey || event.altKey) return;
    if (event.key.length === 1) {
      input({ kind: "text", text: event.key });
    } else if (["Enter", "Backspace", "Escape", "Delete", "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight"].includes(event.key)) {
      input({ kind: "key", key: event.key });
    } else return;
    event.preventDefault();
  }

  function submit() {
    if (!entry.value) return;
    input({ kind: "text", text: entry.value });
    entry.value = "";
  }

  return { state, needsAccessibility, entry, bind, bindNative, nativeEvent, reset, request, cancel, decide, androidState, down, up, wheel, key, submit,
    clearPointer: () => { pointerStart = undefined; } };
}
