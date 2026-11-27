<script setup lang="ts">
import { computed, ref } from "vue";
import type { ConflictAction, FileMeta, ReceiveConflict } from "./types";

const props = defineProps<{
  conflicts: ReceiveConflict[];
  files: FileMeta[];
  directory?: string;
  canSkip: boolean;
  formatSize: (bytes: number) => string;
}>();
const emit = defineEmits<{
  confirm: [actions: Record<string, ConflictAction>];
  cancel: [];
}>();
const actions = ref<Record<string, ConflictAction>>({});
const complete = computed(() => props.conflicts.every(c => actions.value[c.key]));
const skipAll = computed(() => props.files.length === props.conflicts.length &&
  props.conflicts.every(c => actions.value[c.key] === "skip"));
const options = [
  { value: "keep_both", label: "保留两份" },
  { value: "replace", label: "覆盖原文件" },
  { value: "skip", label: "跳过" },
] as const;
function available(action: ConflictAction, conflict: ReceiveConflict) {
  return (action !== "replace" || conflict.replaceable) && (action !== "skip" || props.canSkip);
}
function setAll(event: Event) {
  const select = event.target as HTMLSelectElement;
  const value = select.value as ConflictAction;
  for (const c of props.conflicts) if (available(value, c)) actions.value[c.key] = value;
  select.value = "";
}
</script>

<template>
  <h2 id="modal-title">接收文件时遇到重名</h2>
  <p v-if="directory" class="conflict-destination">目标目录 · {{ directory }}</p>
  <label v-if="conflicts.length > 1" class="conflict-bulk">
    {{ conflicts.length }} 个重名文件
    <select aria-label="批量处理同名文件" @change="setAll">
      <option value="">统一设置…</option>
      <option v-for="o in options" :key="o.value" :value="o.value"
        :disabled="!conflicts.every(c => available(o.value, c))">{{ o.label }}</option>
    </select>
  </label>
  <div class="conflict-files">
    <div v-for="c in conflicts" :key="c.key" class="conflict-file">
      <strong>{{ c.name }}</strong>
      <small>{{ c.reason }}<template v-if="c.existing_size != null"> · {{ formatSize(c.existing_size) }}</template></small>
      <small>接收文件 · {{ formatSize(files.find(f => f.id === c.id)!.size) }}</small>
      <select v-if="conflicts.length > 1" v-model="actions[c.key]" :aria-label="`${c.name} 的处理方式`">
        <option :value="undefined" disabled>选择处理方式…</option>
        <option v-for="o in options" :key="o.value" :value="o.value" :disabled="!available(o.value, c)">{{ o.label }}</option>
      </select>
      <div v-else class="conflict-buttons">
        <button v-for="o in options" :key="o.value" class="secondary" :disabled="!available(o.value, c)"
          @click="emit('confirm', { [c.key]: o.value })">{{ o.label }}</button>
      </div>
    </div>
  </div>
  <p v-if="!canSkip">发送方仅支持整批接收，可返回后另选目录。</p>
  <div class="modal-actions">
    <button class="secondary" @click="emit('cancel')">返回</button>
    <button v-if="conflicts.length > 1" class="primary" :disabled="!complete" @click="emit('confirm', actions)">
      {{ skipAll ? "跳过全部文件" : "继续接收" }}
    </button>
  </div>
</template>

<style scoped>
.conflict-destination, .conflict-file strong { overflow-wrap: anywhere; }
.conflict-bulk, .conflict-buttons { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; }
.conflict-bulk { justify-content: space-between; margin-top: 16px; }
.conflict-bulk select { width: auto; }
.conflict-files { max-height: 46vh; overflow-y: auto; }
.conflict-file { display: grid; gap: 5px; padding: 14px 0; }
.conflict-file + .conflict-file { border-top: 1px solid #e8ecf4; }
.conflict-file small { color: #516078; }
.conflict-file select, .conflict-buttons { margin-top: 6px; }
</style>
