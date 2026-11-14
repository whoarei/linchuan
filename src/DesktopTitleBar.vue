<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { invoke, isTauri } from "@tauri-apps/api/core";
import {
  getCurrentWindow,
  type Window as DesktopWindow,
} from "@tauri-apps/api/window";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { Copy, Minus, Square, X } from "lucide-vue-next";

const emit = defineEmits<{ error: [message: string] }>();
type ResizeDirection = Parameters<DesktopWindow["startResizeDragging"]>[0];
const visible = ref(false);
const maximized = ref(false);
const focused = ref(true);
const busy = ref(false);
const edges: ResizeDirection[] = [
  "North",
  "South",
  "East",
  "West",
  "NorthEast",
  "NorthWest",
  "SouthEast",
  "SouthWest",
];
let appWindow: DesktopWindow | undefined;
let disposed = false;
let revision = 0;
const unlisteners: UnlistenFn[] = [];

function reportError(error: unknown) {
  if (!disposed) emit("error", `窗口操作失败：${String(error)}`);
}
async function syncMaximized() {
  const current = ++revision;
  const value = await appWindow!.isMaximized();
  if (!disposed && current === revision) maximized.value = value;
}
async function track(listener: Promise<UnlistenFn>) {
  const unlisten = await listener;
  if (disposed) unlisten();
  else unlisteners.push(unlisten);
}
async function action(
  operation: () => Promise<void>,
  refreshMaximized = false,
) {
  if (busy.value || !appWindow) return;
  busy.value = true;
  try {
    await operation();
    if (refreshMaximized) await syncMaximized();
  } catch (error) {
    reportError(error);
  } finally {
    busy.value = false;
  }
}
function toggleMaximize() {
  void action(() => appWindow!.toggleMaximize(), true);
}
function drag(event: MouseEvent) {
  if (event.button !== 0 || !appWindow) return;
  event.preventDefault();
  if (event.detail === 2) toggleMaximize();
  else void appWindow.startDragging().catch(reportError);
}
function resize(event: MouseEvent, direction: ResizeDirection) {
  if (event.button !== 0 || !appWindow || maximized.value) return;
  event.preventDefault();
  event.stopPropagation();
  void appWindow.startResizeDragging(direction).catch(reportError);
}
onMounted(async () => {
  if (!isTauri()) return;
  try {
    if (!(await invoke<boolean>("desktop_window_controls")) || disposed) return;
    appWindow = getCurrentWindow();
    visible.value = true;
    document.documentElement.classList.add("desktop-window");
    await Promise.all([
      track(appWindow.onResized(() => void syncMaximized().catch(reportError))),
      track(
        appWindow.onFocusChanged(({ payload }) => (focused.value = payload)),
      ),
    ]);
    await Promise.all([
      syncMaximized(),
      appWindow.isFocused().then((value) => {
        if (!disposed) focused.value = value;
      }),
    ]);
  } catch (error) {
    reportError(error);
  }
});
onUnmounted(() => {
  disposed = true;
  for (const unlisten of unlisteners) unlisten();
  document.documentElement.classList.remove("desktop-window");
});
</script>

<template>
  <template v-if="visible">
    <header
      class="desktop-titlebar"
      :class="{ inactive: !focused }"
      aria-label="窗口标题栏"
    >
      <div class="titlebar-drag" @mousedown="drag">
        <span>邻传 · Linchuan</span>
      </div>
      <div class="window-controls" role="group" aria-label="窗口控制">
        <button
          aria-label="最小化窗口"
          title="最小化窗口"
          :disabled="busy"
          @click="action(() => appWindow!.minimize())"
        >
          <Minus :size="15" :stroke-width="1.5" />
        </button>
        <button
          :aria-label="maximized ? '还原窗口' : '最大化窗口'"
          :title="maximized ? '还原窗口' : '最大化窗口'"
          :aria-pressed="maximized"
          :disabled="busy"
          @click="toggleMaximize"
        >
          <Copy v-if="maximized" :size="13" :stroke-width="1.5" /><Square
            v-else
            :size="13"
            :stroke-width="1.5"
          />
        </button>
        <button
          class="window-close"
          aria-label="关闭窗口"
          title="关闭窗口"
          :disabled="busy"
          @click="action(() => appWindow!.close())"
        >
          <X :size="16" :stroke-width="1.5" />
        </button>
      </div>
    </header>
    <template v-if="!maximized">
      <div
        v-for="edge in edges"
        :key="edge"
        class="window-resize-edge"
        :class="`resize-${edge.toLowerCase()}`"
        aria-hidden="true"
        @mousedown="resize($event, edge)"
      />
    </template>
  </template>
</template>

<style scoped>
.desktop-titlebar {
  position: fixed;
  inset: 0 0 auto;
  height: var(--titlebar-height);
  display: flex;
  background: #f6f7f9;
  border-bottom: 1px solid #e5e8ef;
  z-index: 50;
  user-select: none;
}
.titlebar-drag {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  padding-left: 14px;
  color: #657189;
  font-size: 12px;
}
.titlebar-drag span {
  pointer-events: none;
}
.inactive .titlebar-drag {
  color: #9aa6bd;
}
.window-controls {
  display: flex;
  height: 100%;
}
.window-controls button {
  width: 46px;
  height: 100%;
  border-radius: 0;
  color: #56647c;
  padding: 0;
}
.window-controls button:hover {
  background: #e6e9ef;
}
.window-controls button:focus-visible {
  outline-offset: -3px;
}
.window-controls .window-close:hover {
  color: white;
  background: #d83d48;
}
.window-resize-edge {
  position: fixed;
  z-index: 60;
}
.resize-north,
.resize-south {
  height: 4px;
  left: 7px;
  right: 7px;
  cursor: ns-resize;
}
.resize-north {
  top: 0;
}
.resize-south {
  bottom: 0;
}
.resize-east,
.resize-west {
  width: 4px;
  top: 7px;
  bottom: 7px;
  cursor: ew-resize;
}
.resize-east {
  right: 0;
}
.resize-west {
  left: 0;
}
.resize-northeast,
.resize-northwest,
.resize-southeast,
.resize-southwest {
  width: 7px;
  height: 7px;
}
.resize-northeast,
.resize-northwest {
  top: 0;
}
.resize-southeast,
.resize-southwest {
  bottom: 0;
}
.resize-northeast,
.resize-southeast {
  right: 0;
}
.resize-northwest,
.resize-southwest {
  left: 0;
}
.resize-northeast,
.resize-southwest {
  cursor: nesw-resize;
}
.resize-northwest,
.resize-southeast {
  cursor: nwse-resize;
}
</style>
