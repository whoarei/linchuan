<script setup lang="ts">
import { FileText, LayoutGrid, X } from "lucide-vue-next";

defineProps<{
  windowOnly: boolean;
  canOperate: boolean;
}>();

const note = defineModel<string>("note", { required: true });
const noteOpen = defineModel<boolean>("noteOpen", { required: true });
</script>

<template>
  <div class="demo-desktop" :class="{ 'window-only': windowOnly }">
    <button
      v-if="!windowOnly"
      class="demo-shortcut"
      :disabled="!canOperate"
      @click="noteOpen = true"
    >
      <FileText :size="23" />协作便笺
    </button>
    <div v-if="windowOnly || noteOpen" class="demo-window">
      <header>
        <FileText :size="15" />
        <b>协作便笺</b>
        <button
          v-if="!windowOnly"
          :disabled="!canOperate"
          aria-label="关闭便笺"
          @click="noteOpen = false"
        >
          <X :size="15" />
        </button>
      </header>
      <textarea
        v-model="note"
        aria-label="协作便笺内容"
        :readonly="!canOperate"
        maxlength="400"
        spellcheck="false"
      ></textarea>
    </div>
    <div v-if="!windowOnly" class="demo-taskbar" aria-hidden="true">
      <LayoutGrid :size="16" />
      <FileText :size="15" />
    </div>
  </div>
</template>

<style scoped>
.demo-desktop {
  position: relative;
  display: grid;
  place-items: center;
  isolation: isolate;
  width: min(100%, 760px);
  min-height: 280px;
  aspect-ratio: 16 / 9;
  overflow: hidden;
  border-radius: 11px;
  background: linear-gradient(151deg, #233e76 4%, #4d79bb 58%, #a7bce4 100%);
  box-shadow: 0 12px 28px #17274825;
}
.demo-desktop::before,
.demo-desktop::after {
  position: absolute;
  z-index: -1;
  content: "";
  width: 100%;
  height: 88%;
  border-radius: 52% 52% 0 0;
  transform: rotate(-32deg);
  pointer-events: none;
}
.demo-desktop::before {
  inset: 40% auto auto 33%;
  background: linear-gradient(130deg, #8fafeb, #3766b3 65%);
  box-shadow: inset 0 2px 5px #d6e5ff80;
}
.demo-desktop::after {
  inset: 64% auto auto -20%;
  background: linear-gradient(122deg, #bfcef0, #6088ce 64%);
  box-shadow: inset 0 1px 3px #dae4fa95;
}
.demo-shortcut {
  position: absolute;
  top: 14px;
  left: 12px;
  display: grid;
  justify-items: center;
  gap: 5px;
  padding: 6px;
  border-radius: 6px;
  color: #fff;
  font-size: 11px;
}
.demo-shortcut:enabled:hover { background: #ffffff24; }
.demo-shortcut:disabled,
.demo-window header button:disabled { opacity: 1; cursor: default; }
.demo-window {
  display: flex;
  flex-direction: column;
  width: min(65%, 390px);
  height: 60%;
  min-height: 152px;
  margin-bottom: 22px;
  overflow: hidden;
  border: 1px solid #c9d6ed;
  border-radius: 9px;
  background: #fff;
  box-shadow: 0 14px 30px #15274a50;
}
.demo-window header {
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 35px;
  padding: 0 9px 0 12px;
  border-bottom: 1px solid #e4eaf4;
  background: #f2f5fb;
  color: #405575;
  font-size: 11px;
}
.demo-window header button {
  margin-left: auto;
  padding: 4px;
  border-radius: 4px;
}
.demo-window header button:enabled:hover { background: #e0e8f6; }
.demo-window textarea {
  flex: 1;
  width: 100%;
  min-height: 0;
  resize: none;
  padding: 13px 14px;
  border: 0;
  background: #fff;
  color: #253752;
  font-size: 12px;
  line-height: 1.65;
}
.demo-window textarea:focus-visible { box-shadow: inset 0 0 0 2px #809be7; }
.demo-taskbar {
  position: absolute;
  inset: auto 0 0;
  display: flex;
  justify-content: center;
  align-items: center;
  gap: 19px;
  height: 32px;
  background: #213a66e8;
  color: #e8f0ff;
}
.window-only {
  background: transparent;
  box-shadow: none;
}
.window-only::before,
.window-only::after { display: none; }
.window-only .demo-window {
  width: min(100%, 610px);
  height: 86%;
  margin: 0;
  box-shadow: 0 12px 28px #17274825;
}
@media (max-width: 520px) {
  .demo-shortcut { top: 8px; left: 6px; }
  .demo-window { width: 72%; margin-top: 35px; }
  .demo-window textarea { font-size: 16px; }
  .window-only .demo-window { margin-top: 0; }
}
</style>
