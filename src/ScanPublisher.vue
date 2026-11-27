<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, reactive, ref, watch } from "vue";
import { ArrowLeft, ChevronRight, Copy, FolderOpen, Inbox, Paperclip, Plus, QrCode, Send, X } from "lucide-vue-next";
import { invoke } from "@tauri-apps/api/core";
import QRCode from "qrcode";
import { api } from "./api";
import { scanSize, type ScanKind, type ScanPage } from "./scan";
import type { NativeSelection } from "./types";

const props = defineProps<{ addresses: string[] }>();
const emit = defineEmits<{ copy: [text: string] }>();
const actions = [
  { kind: "pickup", name: "发资料", description: "让访客扫码下载文件、图片或打开链接。", icon: Send },
  { kind: "dropoff", name: "收文件", description: "让访客扫码交件，文件统一保存到接收目录。", icon: Inbox },
] as const;

function emptyDraft() {
  return { title: "", note: "", minutes: 60, links: "", files: [] as NativeSelection[] };
}

const drafts = reactive({ pickup: emptyDraft(), dropoff: emptyDraft() });
const pages = ref<ScanPage[]>([]);
const kind = ref<ScanKind>();
const stage = ref<"create" | "list" | "detail">("create");
const selected = ref("");
const busy = ref(false);
const error = ref("");
const listError = ref("");
const listLoaded = ref(false);
const qr = ref("");
const address = ref("");
const pageHeading = ref<HTMLElement>();
const titleInput = ref<HTMLInputElement>();
const copyAddress = ref<HTMLInputElement>();
let poll: ReturnType<typeof setInterval>;
let refreshing = false;
let revision = 0;

const draft = computed(() => drafts[kind.value ?? "pickup"]);
const pickup = computed(() => kind.value === "pickup");
const newLabel = computed(() => pickup.value ? "新建取件页" : "新建交件页");
const recordsLabel = computed(() => pickup.value ? "取件记录" : "交件记录");
const codeLabel = computed(() => pickup.value ? "取件码" : "交件码");
const matching = computed(() => pages.value.filter((item) => item.kind === kind.value));
const page = computed(() => matching.value.find((item) => item.id === selected.value));
const visitorUrl = computed(() => page.value?.active && address.value
  ? `${address.value}/scan/${page.value.id}` : "");
const directoryLabel = computed(() => {
  const directory = page.value?.directory || "";
  if (directory === "@downloads") return "系统下载目录 / 邻传";
  if (directory.startsWith("content://")) return "Android 自选接收目录";
  return directory;
});

watch(() => props.addresses, (addresses) => {
  if (!addresses.includes(address.value)) address.value = addresses[0] || "";
}, { immediate: true });

watch(visitorUrl, async (value, _, onCleanup) => {
  let stale = false;
  onCleanup(() => { stale = true; });
  qr.value = "";
  if (!value) return;
  try {
    const image = await QRCode.toDataURL(value, { width: 240, margin: 4 });
    if (!stale) qr.value = image;
  } catch {
    if (!stale) error.value = "二维码生成失败，请复制访问链接";
  }
});

async function show(next: typeof stage.value) {
  stage.value = next;
  error.value = "";
  await nextTick();
  if (next === "create") titleInput.value?.focus();
  else pageHeading.value?.focus();
}

function backToActions() {
  kind.value = undefined;
  selected.value = "";
  error.value = "";
  void nextTick(() => pageHeading.value?.focus());
}

function openPage(id: string) {
  selected.value = id;
  void show("detail");
}

function start(next: ScanKind) {
  kind.value = next;
  const active = pages.value.find((item) => item.kind === next && item.active);
  if (active) openPage(active.id);
  else void show("create");
}

async function refresh() {
  if (refreshing || busy.value) return;
  refreshing = true;
  const version = revision;
  try {
    const result = await api<ScanPage[]>("/api/scan");
    if (version !== revision) return;
    pages.value = result;
    listLoaded.value = true;
    listError.value = "";
    if (kind.value && stage.value === "detail" && !page.value) void show("list");
  } catch (failure) {
    if (version === revision) listError.value = (failure as Error).message;
  } finally {
    refreshing = false;
  }
}

function showRecords() {
  void show("list");
  void refresh();
}

async function pickFiles() {
  if (busy.value) return;
  busy.value = true;
  error.value = "";
  const selectedFiles = draft.value.files;
  try {
    const picks = await invoke<NativeSelection[]>("choose_send_files");
    if (picks.length + selectedFiles.length > 64) {
      error.value = "每页最多添加 64 个文件";
      return;
    }
    selectedFiles.push(...picks);
  } catch (failure) {
    error.value = String(failure);
  } finally {
    busy.value = false;
  }
}

async function publish() {
  const currentKind = kind.value;
  if (busy.value || !currentKind) return;
  busy.value = true;
  revision++;
  error.value = "";
  try {
    const currentDraft = drafts[currentKind];
    const result = await api<ScanPage>("/api/scan", {
      kind: currentKind,
      title: currentDraft.title,
      note: currentDraft.note,
      minutes: currentDraft.minutes,
      links: currentDraft.links.split(/\r?\n/).map((link) => link.trim()).filter(Boolean),
      sources: currentDraft.files.map((file) => file.source_id),
    });
    pages.value.unshift(result);
    drafts[currentKind] = emptyDraft();
    openPage(result.id);
  } catch (failure) {
    error.value = (failure as Error).message;
  } finally {
    busy.value = false;
  }
}

async function closePage() {
  if (busy.value || !page.value?.active) return;
  busy.value = true;
  revision++;
  error.value = "";
  try {
    const result = await api<ScanPage>(`/api/scan/${page.value.id}/close`, {});
    pages.value = pages.value.map((item) => item.id === result.id ? result : item);
  } catch (failure) {
    error.value = (failure as Error).message;
  } finally {
    busy.value = false;
  }
}

async function openDirectory() {
  try {
    await invoke("open_scan_directory", { id: selected.value });
  } catch (failure) {
    error.value = String(failure);
  }
}

onMounted(() => {
  void refresh();
  poll = setInterval(() => {
    if (!kind.value || stage.value !== "create") void refresh();
  }, 4000);
});
onUnmounted(() => {
  clearInterval(poll);
  revision++;
});
</script>

<template>
  <main class="scan-workspace" aria-label="收发">
    <div class="scan-content">
      <template v-if="!kind">
        <header class="section-header scan-header">
          <h1 ref="pageHeading" tabindex="-1">收发</h1>
          <p>同一局域网内，访客用手机浏览器扫码即可打开。</p>
        </header>
        <div class="scan-actions">
          <button
            v-for="action in actions"
            :key="action.kind"
            type="button"
            class="scan-action"
            :aria-label="action.name"
            :aria-describedby="`scan-action-${action.kind}`"
            @click="start(action.kind)"
          >
            <span class="scan-action-top">
              <span class="scan-action-icon" :class="{ 'scan-action-inbox': action.kind === 'dropoff' }">
                <component :is="action.icon" :size="20" />
              </span>
              <ChevronRight :size="17" />
            </span>
            <strong>{{ action.name }}</strong>
            <span :id="`scan-action-${action.kind}`">{{ action.description }}</span>
            <small v-if="pages.some((item) => item.kind === action.kind && item.active)" class="scan-open">页面开放中</small>
          </button>
        </div>
      </template>
      <template v-else>
        <div class="scan-toolbar">
          <button type="button" class="text-button scan-back" aria-label="返回收发" :disabled="busy" @click="backToActions">
            <ArrowLeft :size="16" />收发
          </button>
          <button v-if="stage !== 'list' && (matching.length || listError)" type="button" class="text-button" :disabled="busy" @click="showRecords">
            {{ recordsLabel }}
          </button>
        </div>

        <template v-if="stage === 'create'">
          <header class="section-header scan-header"><h1 ref="pageHeading" tabindex="-1">{{ newLabel }}</h1></header>
          <form class="scan-surface scan-form scan-create" @submit.prevent="publish">
            <div class="scan-fields">
              <label>页面标题<input ref="titleInput" v-model.trim="draft.title" maxlength="64" required :placeholder="pickup ? '例如 会议资料' : '例如 活动照片收集'" /></label>
              <label>有效期
                <select v-model.number="draft.minutes">
                  <option :value="15">15 分钟</option>
                  <option :value="60">1 小时</option>
                  <option :value="480">8 小时</option>
                  <option :value="1440">24 小时</option>
                </select>
              </label>
            </div>
            <label>给访客的说明（选填）<textarea v-model="draft.note" maxlength="2000" rows="2" /></label>
            <template v-if="pickup">
              <div>
                <div class="scan-row">
                  <h2>资料</h2>
                  <button type="button" class="secondary" :disabled="busy" @click="pickFiles"><Paperclip :size="16" />添加文件或图片</button>
                </div>
                <ul v-if="draft.files.length" class="scan-list">
                  <li v-for="(file, index) in draft.files" :key="file.source_id">
                    <span><strong>{{ file.name }}</strong><small>{{ scanSize(file.size) }}</small></span>
                    <button type="button" class="icon-button" :disabled="busy" :aria-label="`移除 ${file.name}`" @click="draft.files.splice(index, 1)"><X :size="15" /></button>
                  </li>
                </ul>
              </div>
              <label>链接（每行一个，选填）<textarea v-model="draft.links" rows="2" maxlength="25000" placeholder="https://…" /></label>
            </template>
            <p v-else class="scan-directory">收到的文件会保存到本机接收目录。</p>
            <div class="scan-row">
              <button class="primary" type="submit" :disabled="busy || (pickup && !draft.files.length && !draft.links.trim())">
                <QrCode :size="16" />{{ busy ? '请稍候…' : `生成${codeLabel}` }}
              </button>
            </div>
          </form>
        </template>

        <template v-else-if="stage === 'list'">
          <header class="section-header scan-header"><h1 ref="pageHeading" tabindex="-1">{{ recordsLabel }}</h1></header>
          <section class="scan-surface">
            <p v-if="!listLoaded && !listError" role="status">正在读取页面…</p>
            <p v-else-if="!matching.length && !listError">暂无页面</p>
            <ul v-if="matching.length" class="scan-list">
              <li v-for="item in matching" :key="item.id">
                <button class="scan-page-row" type="button" @click="openPage(item.id)">
                  <span>
                    <strong>{{ item.title }}</strong>
                    <small>{{ item.active ? '开放中' : '已结束' }} · {{ pickup ? `${item.files.length} 个文件 · ${item.links.length} 个链接` : `${item.submissions.length} 个文件` }}</small>
                  </span>
                  <ChevronRight :size="17" />
                </button>
              </li>
            </ul>
            <p v-if="listError" class="error" role="alert">{{ listError }}</p>
            <div class="scan-row scan-section">
              <button class="primary" type="button" @click="show('create')"><Plus :size="16" />{{ newLabel }}</button>
              <button v-if="listError" class="secondary" type="button" @click="refresh">重试</button>
            </div>
          </section>
        </template>

        <template v-else-if="page">
          <header class="section-header scan-header">
            <h1 ref="pageHeading" tabindex="-1">{{ page.title }}</h1>
            <p v-if="page.active" class="scan-open">页面已开放</p>
            <p v-else>页面已结束</p>
          </header>
          <template v-if="page.active">
            <section class="scan-surface scan-qr-panel" :aria-label="codeLabel">
              <img v-if="qr" :src="qr" class="scan-qr" :alt="pickup ? '扫码取件二维码' : '扫码交件二维码'" />
              <div class="scan-share">
                <div>
                  <h2>{{ codeLabel }}</h2>
                  <p class="scan-instruction">同一局域网内，扫码或打开链接。</p>
                  <small>到期时间 {{ new Date(page.expires_at * 1000).toLocaleString("zh-CN", { hour12: false }) }}</small>
                </div>
                <select v-if="addresses.length > 1" v-model="address" aria-label="选择网络地址">
                  <option v-for="item in addresses" :key="item">{{ item }}</option>
                </select>
                <div class="scan-copy">
                  <input ref="copyAddress" readonly :value="visitorUrl" aria-label="访问链接" @focus="copyAddress?.select()" />
                  <button type="button" class="secondary" :disabled="!visitorUrl" @click="emit('copy', visitorUrl)"><Copy :size="15" />复制链接</button>
                </div>
              </div>
            </section>
            <div class="scan-row">
              <button type="button" class="secondary scan-close-page" :disabled="busy" @click="closePage">关闭扫码页面</button>
            </div>
          </template>
          <div v-else class="scan-row">
            <button type="button" class="primary" @click="show('create')"><Plus :size="16" />{{ newLabel }}</button>
          </div>
          <section v-if="!pickup" class="scan-surface" aria-label="收件记录">
            <div class="scan-row">
              <h2>已收文件 <span>{{ page.submissions.length }}</span></h2>
              <button type="button" class="secondary" @click="openDirectory"><FolderOpen :size="16" />打开收件目录</button>
            </div>
            <p class="scan-directory">{{ directoryLabel }}</p>
            <ul v-if="page.submissions.length" class="scan-list">
              <li v-for="item in page.submissions" :key="`${item.name}:${item.submitted_at}`">
                <span><strong>{{ item.name }}</strong><small>{{ item.sender }} · {{ scanSize(item.size) }} · {{ new Date(item.submitted_at).toLocaleTimeString("zh-CN", { hour12: false }) }}</small></span>
              </li>
            </ul>
            <p v-else class="scan-empty">还没有收到文件</p>
          </section>
        </template>
        <p v-if="error" class="error" role="alert">{{ error }}</p>
      </template>
    </div>
  </main>
</template>
