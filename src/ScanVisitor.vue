<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { ArrowUpRight, Check, Download, File, Image as ImageIcon, Link, UploadCloud, X } from "lucide-vue-next";
import { api, ApiError, upload } from "./api";
import { scanId, scanSize, type ScanView } from "./scan";

const props = defineProps<{ scanKey: string }>();
const endpoint = `/api/scan/visit/${props.scanKey}`;
const page = ref<ScanView>();
const loading = ref(true);
const unavailable = ref(false);
const warning = ref("");
const sender = ref("");
const sending = ref(false);
interface Selection {
  id: string;
  file: File;
  bytes: number;
  done: boolean;
  error: string;
}
const files = ref<Selection[]>([]);
const remaining = computed(() => files.value.filter((selection) => !selection.done));
let poll: ReturnType<typeof setInterval>;
let controller: AbortController | undefined;
let refreshing = false;
const anonymous = { anonymous: true };

async function refresh() {
  if (unavailable.value || refreshing) return;
  if (!props.scanKey) {
    unavailable.value = true;
    loading.value = false;
    return;
  }
  refreshing = true;
  try {
    page.value = await api<ScanView>(endpoint, undefined, anonymous);
    document.title = `${page.value.title} · 邻传`;
  } catch (failure) {
    if (failure instanceof ApiError && failure.status === 410) {
      unavailable.value = true;
      controller?.abort();
    } else if (!page.value) {
      warning.value = (failure as Error).message;
    }
  } finally {
    refreshing = false;
    loading.value = false;
  }
}

function selectFiles(event: Event) {
  const input = event.target as HTMLInputElement;
  const choices = Array.from(input.files || []);
  input.value = "";
  warning.value = "";
  if (files.value.length + choices.length > 64) {
    warning.value = "一次最多添加 64 个文件";
    return;
  }
  files.value.push(...choices.map((file) => ({ id: scanId(), file, bytes: 0, done: false, error: "" })));
}

async function submit() {
  if (sending.value || unavailable.value || !remaining.value.length) return;
  sending.value = true;
  warning.value = "";
  controller = new AbortController();
  const { signal } = controller;
  try {
    for (const selection of remaining.value) {
      if (signal.aborted) break;
      selection.error = "";
      selection.bytes = 0;
      const query = new URLSearchParams({
        name: selection.file.name,
        size: String(selection.file.size),
        sender: sender.value.trim(),
      });
      try {
        await upload(`${endpoint}/files/${selection.id}?${query}`, selection.file,
          (bytes) => { selection.bytes = bytes; }, signal, anonymous);
        selection.done = true;
      } catch (failure) {
        selection.error = (failure as Error).message;
        await refresh();
      }
    }
  } finally {
    sending.value = false;
    controller = undefined;
  }
}

onMounted(() => {
  void refresh();
  poll = setInterval(() => { void refresh(); }, 4000);
});
onUnmounted(() => {
  clearInterval(poll);
  controller?.abort();
});
</script>

<template>
  <main class="pair-page scan-visitor">
    <section class="pair-card scan-visitor-card">
      <header class="scan-visitor-header">
        <div class="brand-icon" aria-hidden="true">邻</div>
        <span>{{ page?.kind === 'pickup' ? '扫码取件' : page?.kind === 'dropoff' ? '扫码交件' : '邻传' }}</span>
      </header>
      <h1>{{ page?.title || '邻传扫码页' }}</h1>
      <p v-if="loading" role="status">正在打开页面…</p>
      <p v-else-if="unavailable" class="error" role="alert">此扫码页面已关闭或已过期。</p>
      <template v-else-if="!page">
        <p class="error" role="alert">{{ warning }}</p>
        <button class="secondary scan-section" type="button" @click="refresh">重试</button>
      </template>
      <template v-else>
        <p v-if="page.note" class="scan-note">{{ page.note }}</p>
        <template v-if="page.kind === 'pickup'">
          <section v-if="page.files.length" class="scan-section">
            <h2>文件</h2>
            <ul class="scan-list">
              <li v-for="file in page.files" :key="file.id">
                <a class="scan-file-link" :href="`${endpoint}/files/${file.id}`" :aria-label="`下载 ${file.name}`" download>
                  <component :is="file.mime.startsWith('image/') ? ImageIcon : File" :size="19" />
                  <span><strong>{{ file.name }}</strong><small>{{ scanSize(file.size) }}</small></span>
                  <Download :size="18" />
                </a>
              </li>
            </ul>
          </section>
          <section v-if="page.links.length" class="scan-section">
            <h2>链接</h2>
            <ul class="scan-list">
              <li v-for="(link, index) in page.links" :key="index">
                <a class="scan-file-link" :href="link" target="_blank" rel="noopener noreferrer">
                  <Link :size="18" /><span>{{ link }}</span><ArrowUpRight :size="16" />
                </a>
              </li>
            </ul>
          </section>
        </template>
        <form v-else class="scan-form" @submit.prevent="submit">
          <label>提交人（选填）<input v-model="sender" maxlength="64" autocomplete="name" :disabled="sending" /></label>
          <label>选择要提交的文件或照片<input type="file" multiple :disabled="sending" @change="selectFiles" /></label>
          <ul v-if="files.length" class="scan-list">
            <li v-for="(selection, index) in files" :key="selection.id">
              <span>
                <strong>{{ selection.file.name }}</strong>
                <small v-if="selection.done" class="scan-success"><Check :size="14" />已提交</small>
                <small v-else-if="selection.error" class="error" role="alert">{{ selection.error }}</small>
                <small v-else>{{ selection.bytes ? `${scanSize(selection.bytes)} / ` : '' }}{{ scanSize(selection.file.size) }}</small>
              </span>
              <button v-if="!sending" type="button" class="icon-button" :aria-label="`移除 ${selection.file.name}`" @click="files.splice(index, 1)"><X :size="15" /></button>
            </li>
          </ul>
          <p v-if="warning" class="error" role="alert">{{ warning }}</p>
          <p v-if="files.some((selection) => selection.done) && !remaining.length" class="scan-success" role="status">本次文件已全部提交。</p>
          <button type="submit" class="primary wide" :disabled="sending || !remaining.length"><UploadCloud :size="17" />{{ sending ? '正在提交…' : '提交文件' }}</button>
        </form>
      </template>
    </section>
  </main>
</template>
