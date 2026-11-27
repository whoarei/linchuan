<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref } from "vue";
import {
  ArrowLeft,
  ArrowUpRight,
  Check,
  CheckCheck,
  ChevronDown,
  ChevronRight,
  Copy,
  Download,
  File,
  FolderOpen,
  FolderInput,
  Image as ImageIcon,
  Info,
  Link,
  LoaderCircle,
  LogOut,
  MessageSquare,
  Monitor,
  MonitorUp,
  MoreHorizontal,
  Paperclip,
  Plus,
  QrCode,
  Search,
  Send,
  Settings,
  ShieldCheck,
  Smartphone,
  Trash2,
  Users,
  Wifi,
  X,
  Globe,
  RefreshCw,
} from "lucide-vue-next";
import QRCode from "qrcode";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import BrandMark from "./BrandMark.vue";
import DesktopTitleBar from "./DesktopTitleBar.vue";
import ScreenAssistance from "./ScreenAssistance.vue";
import ConversationRow from "./ConversationRow.vue";
import ReceiveConflictForm from "./ReceiveConflictForm.vue";
import ScanPublisher from "./ScanPublisher.vue";
import { useUpdates } from "./useUpdates";
import { useReceiving } from "./useReceiving";
import { useDesktopSettings } from "./useDesktopSettings";
import { useModalFocus } from "./useModalFocus";
import {
  browserIdentity,
  rememberBrowserName,
  randomNickname,
  deviceLabels,
} from "./names";
import {
  api,
  bootstrap,
  native,
  token,
  setToken,
  forget,
  pairingCode,
  browserHttps,
  address,
  upload,
  ApiError,
} from "./api";
import type {
  AppState,
  Conversation,
  Device,
  Group,
  Message,
  MessageFile,
  NativeSelection,
  Transfer,
  FileMeta,
  ConflictAction,
  ReceiveConflict,
} from "./types";

type Mode = "devices" | "scan" | "settings" | "about";
const {
  settings: desktopSettings,
  busy: desktopBusy,
  load: loadDesktopSettings,
  setAutostart,
  setCloseToTray,
  quit: quitDesktop,
} = useDesktopSettings(notify);
const downloadsUrl = "https://github.com/whoarei/linchuan/releases/latest";
const {
  settings: receiveSettings,
  busy: receiveBusy,
  fileBusy,
  receipt,
  loadSettings: loadReceiveSettings,
  loadReceipts,
  changeDirectory,
  openDirectory,
  openFile: openSavedFile,
  saveFile,
  relocateFile,
} = useReceiving(notify);
const {
  state: updateState,
  busy: updateBusy,
  message: updateMessage,
  checkForUpdate,
  downloadUpdate,
  installUpdate,
  restartAfterUpdate,
  openUpdatePage,
} = useUpdates();
const state = ref<AppState>();
const loading = ref(true);
const bootError = ref("");
const ready = ref(false);
const connected = ref(true);
const mode = ref<Mode>("devices");
const selected = ref("");
const search = ref("");
const showTemporaryHistory = ref<boolean>();
const messages = ref<Message[]>([]);
const before = ref<number | null>(null);
const historyBusy = ref(false);
const text = ref("");
type SelectedFile = File & { source_id?: string };
const files = ref<SelectedFile[]>([]);
const fileCache = new Map<string, SelectedFile[]>();
const browserOnly = computed(() => state.value?.me.owner === false);
const sendPhase = ref("正在发送…");
const nativeListeners: UnlistenFn[] = [];
const sending = ref(false);
const progress = ref(0);
const uploadTotal = ref(0);
let controller: AbortController | undefined;
const input = ref<HTMLInputElement>();
const composer = ref<HTMLTextAreaElement>();
const timeline = ref<HTMLDivElement>();
const dragging = ref(false);
const pairing = ref(false);
const browserIdentityName = browserIdentity();
const browserName = ref(browserIdentityName.preferred);
const code = ref("");
const hostName = ref("邻传设备");
const pairError = ref("");
const modal = ref<
  | ""
  | "connect"
  | "browser-https"
  | "manual"
  | "group"
  | "members"
  | "clear"
  | "leave"
  | "preview"
  | "receive-conflicts"
>("");
const modalContainer = ref<HTMLElement>();
useModalFocus(modal, modalContainer, closeModal);
const addMenu = ref(false);
const qr = ref("");
const shareAddress = ref("");
const httpsAddressSelected = computed(() => !!browserHttps && shareAddress.value === browserHttps.address);
const shareCode = computed(() => httpsAddressSelected.value ? browserHttps?.pairing_code : pairingCode);
const httpsHost = ref("");
const httpsPort = ref<number | "">("");
const httpsCertificate = ref<File>();
const httpsKey = ref<File>();
const manualAddress = ref("");
const modalBusy = ref(false);
const groupName = ref("");
const memberIds = ref<string[]>([]);
const editingGroup = ref("");
const expanded = ref(new Set<string>());
const newName = ref("");
const nameBusy = ref(false);
const toast = ref("");
const toastError = ref(false);
let toastTimer: ReturnType<typeof setTimeout>;
const preview = ref<{ url: string; name: string }>();
const thumbnails = ref<Record<string, string>>({});
type ReceiveDecision = "reject" | "accept" | "save-as";
const decisionBusy = ref(new Map<string, ReceiveDecision>());
const conflictPrompt = ref<{
  conflicts: ReceiveConflict[];
  files: FileMeta[];
  directory?: string;
  canSkip: boolean;
  resolve: (actions?: Record<string, ConflictAction>) => void;
}>();
function finishConflictPrompt(actions?: Record<string, ConflictAction>) {
  conflictPrompt.value?.resolve(actions);
  conflictPrompt.value = undefined;
  modal.value = "";
}
const excludedFiles = ref(new Map<string, Set<string>>());
type TransferSample = { bytes: number; time: number; rate?: number };
const transferSamples = ref(new Map<string, TransferSample>());
const thumbnailRequests = new Set<string>();
const autoDownloads = new Set<string>();
let thumbnailBytes = 0;
let poll: ReturnType<typeof setInterval>;
let draftTimer: ReturnType<typeof setTimeout>;
let socket: WebSocket | undefined;
let disposed = false;
let refreshing = false;
let timelineRequest = 0;
let markedPosition = 0;
let eventTimer: ReturnType<typeof setTimeout>;
let draftWrites: Promise<unknown> = Promise.resolve();
const current = computed(() =>
  state.value?.conversations.find((c) => c.id === selected.value),
);
const currentDevice = computed(() =>
  state.value?.devices.find((d) => d.id === current.value?.peer_id),
);
const screenAssistance = ref<InstanceType<typeof ScreenAssistance>>();
const displayNames = computed(() => {
  if (!state.value) return new Map<string, string>();
  return deviceLabels([
    ...state.value.groups.flatMap((g) =>
      g.members
        .filter(
          (m) =>
            m.device.kind !== "browser" ||
            online(m.device.id) ||
            g.id === current.value?.group_id,
        )
        .map((m) => m.device),
    ),
    ...state.value.conversations
      .filter(
        (c) =>
          c.kind === "direct" &&
          c.peer_id &&
          (!temporaryOffline(c) ||
            (temporaryHistoryExpanded.value &&
              (c.last_message_at || c.has_draft))),
      )
      .map((c) => ({ id: c.peer_id!, name: c.name })),
    ...state.value.devices,
    state.value.me,
  ]);
});
function deviceLabel(id: string, name: string) {
  return displayNames.value.get(id) || name;
}
function conversationLabel(conversation: Conversation) {
  return conversation.kind === "direct"
    ? deviceLabel(conversation.peer_id || "", conversation.name)
    : conversation.name;
}
function shuffleName() {
  newName.value = randomNickname([newName.value, state.value?.me.name || ""]);
}
const group = computed(() =>
  state.value?.groups.find((g) => g.id === current.value?.group_id),
);
const membership = computed(() =>
  group.value?.members.find((m) => m.device.id === state.value?.me.id),
);
const owner = computed(() => group.value?.owner_id === state.value?.me.id);
const canWrite = computed(
  () =>
    !!current.value &&
    (current.value.kind === "direct" || membership.value?.state === "joined"),
);
const textTooLong = computed(() => new TextEncoder().encode(text.value).length > 65_536);
const canSend = computed(
  () =>
    connected.value &&
    canWrite.value &&
    !sending.value &&
    (text.value.trim().length > 0 || files.value.length > 0) &&
    !textTooLong.value,
);
function browserConversation(c: Conversation) {
  return c.kind === "direct" && c.peer_kind === "browser";
}
function temporaryOffline(c: Conversation) {
  return browserConversation(c) && !online(c.peer_id);
}
function matchesSearch(c: Conversation) {
  return `${c.name} ${c.preview}`
    .toLowerCase()
    .includes(search.value.toLowerCase());
}
const temporaryHistory = computed(
  () =>
    state.value?.conversations.filter(
      (c) =>
        browserConversation(c) &&
        (online(c.peer_id) || c.last_message_at || c.has_draft) &&
        matchesSearch(c),
    ) || [],
);
const temporaryHistoryExpanded = computed(() => !!search.value.trim() ||
  (showTemporaryHistory.value ?? temporaryHistory.value.some((c) => online(c.peer_id))));
const historyUnread = computed(() =>
  temporaryHistory.value.reduce((sum, c) => sum + c.unread, 0),
);
const conversations = computed(
  () =>
    state.value?.conversations.filter(
      (c) => !browserConversation(c) && matchesSearch(c),
    ) || [],
);
const nearby = computed(
  () =>
    state.value?.devices.filter(
      (d) =>
        d.kind !== "browser" &&
        !state.value!.conversations.some((c) => c.peer_id === d.id) &&
        d.name.toLowerCase().includes(search.value.toLowerCase()),
    ) || [],
);
const eligible = computed(
  () =>
    state.value?.devices.filter(
      (d) =>
        d.kind !== "browser" && d.capabilities?.includes("groups-v1") &&
        (!editingGroup.value ||
          !group.value?.members.some(
            (m) => m.device.id === d.id && m.state !== "left",
          )),
    ) || [],
);
const unread = computed(
  () => state.value?.conversations.reduce((n, c) => n + c.unread, 0) || 0,
);
const terminal = (status: string) =>
  ["completed", "rejected", "cancelled", "failed", "interrupted"].includes(
    status,
  );
function notify(message: string, error = false) {
  toast.value = message;
  toastError.value = error;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (toast.value = ""), 4500);
}
function size(bytes: number) {
  if (!bytes) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), 3);
  return `${(bytes / 1024 ** i).toFixed(i ? 1 : 0)} ${units[i]}`;
}
function sampleTransferRates(transfers: Transfer[]) {
  const time = performance.now();
  const samples = new Map<string, TransferSample>();
  for (const transfer of transfers) {
    if (transfer.status !== "accepted" && transfer.status !== "transferring") continue;
    const previous = transferSamples.value.get(transfer.id);
    const bytes = transfer.bytes;
    if (!previous || bytes < previous.bytes) {
      samples.set(transfer.id, { bytes, time });
    } else if (time - previous.time >= 800) {
      const rate = Math.round((bytes - previous.bytes) * 1000 / (time - previous.time));
      samples.set(transfer.id, { bytes, time, rate });
    } else {
      samples.set(transfer.id, previous);
    }
  }
  transferSamples.value = samples;
}
function transferProgress(transfer: Transfer) {
  const rate = transferSamples.value.get(transfer.id)?.rate;
  const amount = `${size(transfer.bytes)} / ${size(transfer.total)}`;
  return transfer.status === "transferring" && rate !== undefined
    ? `${amount} · ${size(rate)}/s`
    : amount;
}
function timestamp(ms: number) {
  const date = new Date(ms);
  const today = new Date();
  return date.toDateString() === today.toDateString()
    ? date.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" })
    : date.toLocaleDateString("zh-CN", { month: "numeric", day: "numeric" });
}
function online(id?: string) {
  return (
    id === state.value?.me.id || (browserOnly.value && id===state.value?.info.id && connected.value) || !!state.value?.devices.some((d) => d.id === id)
  );
}
function summary(message: Message) {
  const ds = message.deliveries;
  const done = ds.filter((d) => d.status === "completed").length;
  if (ds.length && done === ds.length && ds.some((d) => d.selected_file_ids))
    return ds.length === 1 ? `已传送 ${ds[0].selected_file_ids!.length}/${message.files.length} 个文件` : "部分文件送达";
  if (ds.length && done === ds.length) return message.files.length && ds.some((d)=>d.recipient_id.startsWith("browser-")) ? "已发送至浏览器" : "已送达";
  if (done && ds.every((d) => terminal(d.status)))
    return `部分送达 ${done}/${ds.length}`;
  if (ds.some((d) => d.status === "transferring" || d.status === "accepted"))
    return "正在传输";
  if (ds.some((d) => d.status === "pending")) return "等待接收";
  if (ds.some((d) => d.status === "interrupted")) return "传输中断";
  if (ds.every((d) => d.status === "rejected")) return "已拒绝";
  if (ds.every((d) => d.status === "cancelled")) return "已取消";
  return "未送达";
}
function statusLabel(status: string) {
  return (
    (
      {
        pending: "等待接收",
        accepted: "已接受",
        transferring: "正在传输",
        completed: "已完成",
        rejected: "已拒绝",
        cancelled: "已取消",
        failed: "未送达",
        interrupted: "传输中断",
      } as Record<string, string>
    )[status] || status
  );
}
function incomingDelivery(message: Message) {
  return message.deliveries.find((d) => d.recipient_id === state.value?.me.id);
}
function incomingPending(message: Message) {
  const delivery = incomingDelivery(message);
  return delivery?.status === "pending" ? delivery : undefined;
}
function incomingActive(message: Message) {
  const delivery = incomingDelivery(message);
  return delivery && ["accepted", "transferring"].includes(delivery.status)
    ? delivery
    : undefined;
}
function selectedForReceive(transfer: Transfer) {
  return transfer.files.filter((file) => !excludedFiles.value.get(transfer.id)?.has(file.id));
}
function toggleReceiveFile(transfer: Transfer, fileId: string, event: Event) {
  const excluded = new Set(excludedFiles.value.get(transfer.id));
  if ((event.target as HTMLInputElement).checked) excluded.delete(fileId);
  else excluded.add(fileId);
  excludedFiles.value.set(transfer.id, excluded);
}
function includesFile(transfer: Transfer | undefined, fileId: string) {
  return transfer?.selected_file_ids?.includes(fileId) ?? true;
}
function visibleDeliveries(message: Message) {
  return message.deliveries.filter(
    (delivery) =>
      !message.files.length || delivery.id !== incomingActive(message)?.id,
  );
}
function toggleExpanded(id: string) {
  const result = new Set(expanded.value);
  result.has(id) ? result.delete(id) : result.add(id);
  expanded.value = result;
}
function scrollBottom() {
  void nextTick(() => {
    if (timeline.value) timeline.value.scrollTop = timeline.value.scrollHeight;
  });
}
async function refresh(initial = false) {
  if (refreshing || disposed) return;
  refreshing = true;
  const requestToken = token();
  try {
    const next = await api<AppState>("/api/state");
    if (disposed || requestToken !== token()) return;
    sampleTransferRates(next.transfers);
    for (const id of excludedFiles.value.keys()) {
      if (!next.transfers.some((t) => t.id === id && t.status === "pending"))
        excludedFiles.value.delete(id);
    }
    state.value = next;
    if (!next.me.owner) {
      let host=next.conversations.find((c)=>c.kind==="direct" && c.peer_id===next.info.id);
      if(!host) {
        const raw=await api<Conversation>("/api/conversations/direct",{device_id:next.info.id});
        host={...raw,name:next.info.name,preview:"",last_message_at:0,unread:0};
        next.conversations.push(host);
      }
      if(selected.value!==host.id) await selectConversation(host);
    }
    connected.value = true;
    ready.value = true;
    if (!shareAddress.value)
      shareAddress.value = state.value.info.addresses[0] || address();
    if (selected.value && (mode.value === "devices" || !next.me.owner) && !historyBusy.value)
      await loadMessages(false);
  } catch (error) {
    if (disposed || requestToken !== token()) return;
    if (error instanceof ApiError && error.status === 401 && !native) {
      forget(requestToken);
      sessionChanged();
    } else {
      connected.value = false;
      if (initial) throw error;
    }
  } finally {
    if (!initial) loading.value = false;
    refreshing = false;
  }
}
function events() {
  if (
    socket &&
    (socket.readyState === WebSocket.OPEN ||
      socket.readyState === WebSocket.CONNECTING)
  )
    return;
  socket = new WebSocket(address().replace(/^http/, "ws") + "/api/events");
  socket.onopen = () => socket?.send(JSON.stringify({ token: token() }));
  socket.onmessage = () => {
    clearTimeout(eventTimer);
    eventTimer = setTimeout(() => {
      void refresh();
      if (!native) void screenAssistance.value?.refresh();
    }, 120);
  };
  socket.onerror = () => {};
}
async function pair() {
  pairing.value = true;
  pairError.value = "";
  try {
    const name = browserName.value.trim() || browserIdentityName.automatic;
    const value = await api<{ token: string; name?: string }>("/api/pair", {
      name,
      code: code.value,
    });
    setToken(value.token);
    browserName.value = value.name || name;
    rememberBrowserName(browserName.value);
    await refresh();
    if (ready.value) events();
  } catch (error) {
    pairError.value = (error as Error).message;
  } finally {
    pairing.value = false;
  }
}
function releaseThumbnails() {
  for (const url of Object.values(thumbnails.value)) URL.revokeObjectURL(url);
  thumbnails.value = {};
  thumbnailRequests.clear();
  thumbnailBytes = 0;
}
async function selectConversation(c: Conversation) {
  if (selected.value === c.id) {
    mode.value = "devices";
    return;
  }
  flushDraft();
  fileCache.set(selected.value, [...files.value]);
  selected.value = c.id;
  mode.value = "devices";
  messages.value = [];
  before.value = null;
  markedPosition = 0;
  text.value = "";
  files.value = fileCache.get(c.id) || [];
  releaseThumbnails();
  await loadMessages(true);
  scrollBottom();
}
async function selectDevice(device: Device) {
  try {
    const c = await api<Conversation>("/api/conversations/direct", {
      device_id: device.id,
    });
    await refresh();
    await selectConversation(
      state.value!.conversations.find((v) => v.id === c.id) || c,
    );
  } catch (error) {
    notify((error as Error).message, true);
  }
}
async function loadMessages(initial: boolean, older = false) {
  const id = selected.value;
  if (!id) return;
  const request = ++timelineRequest;
  const draftBeforeRequest = text.value;
  if (initial || older) historyBusy.value = true;
  const keepBottom =
    !timeline.value ||
    timeline.value.scrollHeight -
      timeline.value.scrollTop -
      timeline.value.clientHeight <
      100;
  const oldHeight = timeline.value?.scrollHeight || 0;
  try {
    const page = await api<{
      messages: Message[];
      next_before: number | null;
      draft: string;
    }>(
      `/api/conversations/${encodeURIComponent(id)}/messages${older && before.value ? `?before=${before.value}` : ""}`,
    );
    if (id !== selected.value || request !== timelineRequest) return;
    if (older) {
      messages.value = [
        ...page.messages,
        ...messages.value.filter(
          (m) => !page.messages.some((v) => v.id === m.id),
        ),
      ];
      before.value = page.next_before;
      void nextTick(() => {
        if (timeline.value)
          timeline.value.scrollTop = timeline.value.scrollHeight - oldHeight;
      });
    } else {
      const olderMessages = messages.value.filter(
        (m) => page.messages.length && m.position < page.messages[0].position,
      );
      messages.value = [...olderMessages, ...page.messages];
      if (initial || !olderMessages.length) before.value = page.next_before;
      if (initial && text.value === draftBeforeRequest) text.value = page.draft;
      if (initial || keepBottom) scrollBottom();
    }
    const last = messages.value[messages.value.length - 1]?.position || 0;
    if (last > markedPosition && mode.value === "devices") {
      markedPosition = last;
      void api(`/api/conversations/${encodeURIComponent(id)}/read`, {
        through_position: last,
      }).catch(() => {});
    }
    await loadReceipts(messages.value.map((message) => message.id));
    if (id === selected.value && request === timelineRequest)
      void loadThumbnails(id, page.messages);
  } catch (error) {
    if (initial || older) notify((error as Error).message, true);
  } finally {
    if (request === timelineRequest) historyBusy.value = false;
  }
}
function queueDraft() {
  clearTimeout(draftTimer);
  draftTimer = setTimeout(flushDraft, 450);
}
function flushDraft() {
  clearTimeout(draftTimer);
  const id = selected.value;
  const value = text.value;
  if (!id || !ready.value) return;
  draftWrites = draftWrites
    .catch(() => {})
    .then(() =>
      api(`/api/conversations/${encodeURIComponent(id)}/draft`, {
        text: value,
      }),
    )
    .catch(() => {
      if (connected.value) notify("草稿未保存，请检查连接", true);
    });
}
function addFiles(items: SelectedFile[]) {
  if (sending.value || !canWrite.value) return;
  for (const file of items) {
    if (!Number.isSafeInteger(file.size) || file.size < 0) {
      notify(`${file.name} 的大小无法读取`, true);
      continue;
    }
    if (files.value.length >= 64) {
      notify("一次最多添加 64 个文件", true);
      break;
    }
    files.value.push(file);
  }
  fileCache.set(selected.value, [...files.value]);
}
function nativeSelections(items: NativeSelection[]) {
  addFiles(items.map((f) => ({ name:f.name,size:f.size,type:f.mime,source_id:f.source_id }) as SelectedFile));
}
async function chooseFiles() {
  if (!native) { input.value?.click(); return; }
  try { nativeSelections(await invoke<NativeSelection[]>("choose_send_files") || []); }
  catch(error) { notify(String(error),true); }
}
function fileChange(event: Event) {
  const element = event.target as HTMLInputElement;
  addFiles(Array.from(element.files || []));
  element.value = "";
}
function paste(event: ClipboardEvent) {
  const images = Array.from(event.clipboardData?.items || [])
    .filter((i) => i.kind === "file")
    .map((i) => i.getAsFile())
    .filter((f): f is File => f !== null);
  if (images.length) {
    event.preventDefault();
    addFiles(images);
  }
}
function drop(event: DragEvent) {
  event.preventDefault();
  dragging.value = false;
  addFiles(Array.from(event.dataTransfer?.files || []));
}
async function send() {
  if (!canSend.value || !current.value) return;
  const conversation = current.value;
  const body = text.value;
  const selectedFiles = [...files.value];
  clearTimeout(draftTimer);
  await draftWrites;
  sending.value = true;
  progress.value = 0;
  uploadTotal.value = selectedFiles.reduce((n, f) => n + f.size, 0);
  controller = new AbortController();
  const signal = controller.signal;
  let preparedDraft = "";
  let sent = false;
  try {
    const streaming = browserOnly.value;
    sendPhase.value = streaming ? "等待对方接收…" : "正在发送…";
    const draft = await api<{ id: string; files: FileMeta[] }>("/api/drafts", {
      streaming,
      files: selectedFiles.map((f) => ({ name:f.name,size:f.size,mime:f.type,source_id:f.source_id })),
    });
    preparedDraft = draft.id;
    if (!streaming) {
      let count = 0;
      for (let i=0;i<selectedFiles.length;i++) {
        if (!selectedFiles[i].source_id) {
          sendPhase.value="正在保存粘贴内容…";
          await upload("/api/drafts/"+draft.id+"/files/"+draft.files[i].id,selectedFiles[i],(bytes)=>(progress.value=count+bytes),signal);
        }
        count += selectedFiles[i].size;
      }
    }
    if(signal.aborted) throw new Error("已取消发送");
    const result = await api<{ids:string[]}>("/api/drafts/"+draft.id+"/send", {
      text:body, targets:conversation.kind==="direct"?[conversation.peer_id]:[], group_id:conversation.group_id,
    });
    sent=true;
    fileCache.set(conversation.id,[]);
    if(selected.value===conversation.id){text.value="";files.value=[];}
    await refresh();scrollBottom();
    if(streaming && selectedFiles.length) {
      const transferId=result.ids[0];
      try {
        const deadline=Date.now()+5*60_000;
        let accepted: Transfer;
        while(true) {
          if(signal.aborted) throw new Error("已取消发送");
          const next=await api<AppState>("/api/state");
          const transfer=next.transfers.find((t)=>t.id===transferId);
          if(!transfer || terminal(transfer.status)) throw new Error(transfer?.error || (transfer?.status==="rejected"?"对方已拒绝接收":"传输已结束"));
          if (transfer.status === "accepted") { accepted = transfer; break; }
          if(Date.now()>deadline) throw new Error("等待接收超时，请重新发送");
          await new Promise((resolve)=>setTimeout(resolve,400));
        }
        const uploads = draft.files
          .map((meta, index) => ({ id: meta.id, file: selectedFiles[index] }))
          .filter(({ id }) => includesFile(accepted, id))
          .sort((a, b) => a.file.size - b.file.size);
        uploadTotal.value = accepted.total;
        sendPhase.value="正在发送文件…";
        let count=0;
        for (const { id, file } of uploads) {
          await upload(`/api/transfers/${transferId}/files/${id}/upload`, file, (bytes) => (progress.value = count + bytes), signal);
          count += file.size;
        }
      } catch(error) {
        await api("/api/transfers/"+transferId+"/cancel",{}).catch(()=>{});
        throw error;
      }
    }
    await refresh();
    scrollBottom();
  } catch (error) {
    if(browserOnly.value && selectedFiles.length && token()) {
      fileCache.set(conversation.id,selectedFiles);
      if(selected.value===conversation.id && !files.value.length){files.value=selectedFiles;if(!text.value)text.value=body;}
    }
    notify((error as Error).message, true);
  } finally {
    if (preparedDraft && !sent) {
      for (let attempt = 0; attempt < 5; attempt++) {
        try {
          await api(`/api/drafts/${preparedDraft}/cancel`, {});
          break;
        } catch (error) {
          if (!(error instanceof ApiError) || error.status !== 409) break;
          await new Promise((resolve) => setTimeout(resolve, 100));
        }
      }
    }
    sending.value = false;
    controller = undefined;
  }
}
async function decide(transfer: Transfer, decision: ReceiveDecision) {
  if (decisionBusy.value.size) return;
  let selected = selectedForReceive(transfer);
  if (decision !== "reject" && transfer.files.length && !selected.length) return;
  let directory: string | undefined;
  const choices: Record<string, ConflictAction> = {};
  decisionBusy.value.set(transfer.id, decision);
  try {
    while (true) {
      const fileIds = selected.length === transfer.files.length ? undefined : selected.map(f => f.id);
      const accept = decision !== "reject" && (!transfer.files.length || selected.length > 0);
      let conflicts: ReceiveConflict[];
      if (decision === "save-as" && accept) {
        const result = await invoke<boolean | { directory: string; conflicts: ReceiveConflict[] }>("receive_transfer_as", {
          transferId: transfer.id, fileIds, directory, choices,
        });
        if (result === false) return;
        if (result === true) break;
        directory = result.directory;
        conflicts = result.conflicts;
      } else {
        try {
          await api(`/api/transfers/${transfer.id}/decision`, {
            accept, file_ids: accept ? fileIds : undefined, choices,
          });
          break;
        } catch (error) {
          if (!(error instanceof ApiError) || !error.conflicts?.length) throw error;
          conflicts = error.conflicts;
        }
      }
      if (disposed || modal.value) return;
      const actions = await new Promise<Record<string, ConflictAction> | undefined>(resolve => {
        conflictPrompt.value = {
          conflicts, files: selected, resolve,
          canSkip: !!transfer.can_select_files || selected.length === 1,
          directory: directory?.startsWith("content://") ? "所选文件夹" : directory || receiveSettings.value?.label,
        };
        modal.value = "receive-conflicts";
      });
      if (!actions) return;
      for (const c of conflicts) {
        const action = actions[c.key];
        if (action === "skip") selected = selected.filter(f => f.id !== c.id);
        else choices[c.key] = action;
      }
    }
    await refresh();
    if (decision === "accept" && browserOnly.value && transfer.files.length) {
      const message = messages.value.find((m) => m.id === transfer.message_id);
      if (message) {
        const files = message.files.filter((file) => selected.some(f => f.id === file.id));
        files.sort((a, b) => a.size - b.size);
        for (const file of files) await download(file, message.id);
      }
    }
  } catch (error) {
    notify(error instanceof Error ? error.message : String(error), true);
  } finally {
    decisionBusy.value.delete(transfer.id);
  }
}
async function cancel(transfer: Transfer) {
  try {
    await api(`/api/transfers/${transfer.id}/cancel`, {});
    await refresh();
  } catch (error) {
    notify((error as Error).message, true);
  }
}
async function fileUrl(file: MessageFile) {
  const value = await api<{ url: string }>(
    `/api/transfers/${file.transfer_id}/files/${file.id}/ticket`,
    {},
  );
  return address() + value.url;
}
async function download(file: MessageFile, message: string) {
  if (native) {
    await saveFile(message, file);
    return;
  }
  try {
    const url = await fileUrl(file);
    const a = document.createElement("a");
    a.href = url;
    a.download = file.name;
    document.body.append(a);
    a.click();
    a.remove();
  } catch (error) {
    notify((error as Error).message, true);
  }
}
const raster = (file: FileMeta) =>
  /^image\/(png|jpeg|gif|webp|bmp)$/.test(file.mime);
async function blobUrl(file: MessageFile) {
  const response = await fetch(await fileUrl(file));
  if (!response.ok) throw new Error("文件已过期或无法读取");
  return URL.createObjectURL(
    new Blob([await response.blob()], { type: file.mime }),
  );
}
async function loadThumbnails(id: string, list: Message[]) {
  for (const message of list) {
    const delivery = incomingActive(message);
    const automatic = browserOnly.value && delivery &&
      message.files.every((file) => file.mime.startsWith("image/"));
    for (const file of message.files) {
      if (disposed || id !== selected.value) return;
      const key = message.id + file.id;
      if (!file.available || thumbnails.value[key] || thumbnailRequests.has(key)) continue;
      if (!raster(file) || file.size > 4 * 1024 ** 2 ||
          thumbnailBytes + file.size > 32 * 1024 ** 2) {
        // Larger images use the browser download manager instead of an in-memory Blob.
        const downloadKey = delivery?.id + ":" + file.id;
        if (automatic && !autoDownloads.has(downloadKey)) {
          autoDownloads.add(downloadKey);
          await download(file, message.id);
        }
        continue;
      }
      thumbnailRequests.add(key);
      thumbnailBytes += file.size;
      let retained = false;
      try {
        const url = await blobUrl(file);
        if (id !== selected.value || disposed) {
          URL.revokeObjectURL(url);
          continue;
        }
        thumbnails.value[key] = url;
        retained = true;
      } catch {} finally {
        if (id === selected.value && !disposed) {
          thumbnailRequests.delete(key);
          if (!retained) thumbnailBytes -= file.size;
        }
      }
    }
  }
}
async function viewImage(file: MessageFile) {
  if (file.size > 20 * 1024 ** 2) {
    notify("大于 20 MB 的图片请下载后查看");
    return;
  }
  try {
    closePreview();
    preview.value = { url: await blobUrl(file), name: file.name };
    modal.value = "preview";
  } catch (error) {
    notify((error as Error).message, true);
  }
}
function closePreview() {
  if (preview.value) URL.revokeObjectURL(preview.value.url);
  preview.value = undefined;
}
function closeModal() {
  if (conflictPrompt.value) finishConflictPrompt();
  else modal.value = "";
  closePreview();
}
async function copy(value: string) {
  try {
    if (navigator.clipboard && window.isSecureContext)
      await navigator.clipboard.writeText(value);
    else {
      const input = document.createElement("textarea");
      input.value = value;
      input.style.position = "fixed";
      input.style.opacity = "0";
      document.body.append(input);
      input.select();
      const ok = document.execCommand("copy");
      input.remove();
      if (!ok) throw new Error();
    }
    notify("已复制");
  } catch {
    notify("请选中文字后手动复制", true);
  }
}
async function showConnection() {
  addMenu.value = false;
  modal.value = "connect";
  await updateQr();
}
async function updateQr() {
  qr.value = await QRCode.toDataURL(shareAddress.value, {
    width: 240,
    margin: 1,
    color: { dark: "#24334d", light: "#ffffff" },
  });
}
function configureBrowserHttps() {
  httpsHost.value = browserHttps ? new URL(browserHttps.address).hostname : "";
  httpsPort.value = browserHttps ? Number(new URL(browserHttps.address).port) || 443 : "";
  httpsCertificate.value = undefined;
  httpsKey.value = undefined;
  modal.value = "browser-https";
}
function choosePem(event: Event, type: "certificate" | "key") {
  const file = (event.target as HTMLInputElement).files?.[0];
  if (type === "certificate") httpsCertificate.value = file;
  else httpsKey.value = file;
}
async function saveBrowserHttps() {
  const certificate = httpsCertificate.value;
  const key = httpsKey.value;
  if (!certificate || !key || modalBusy.value) return;
  if (certificate.size > 128 * 1024 || key.size > 32 * 1024) {
    notify("证书或私钥文件过大", true);
    return;
  }
  modalBusy.value = true;
  try {
    await invoke("assistance_browser_https_setup", {
      host: httpsHost.value.trim(), port: httpsPort.value || 0,
      certificatePem: await certificate.text(), keyPem: await key.text(),
    });
    await showConnection();
    notify("已保存 HTTPS 配置，完全退出后重新打开邻传即可生效");
  } catch (error) {
    notify(String(error), true);
  } finally {
    modalBusy.value = false;
  }
}
async function connectPeer() {
  modalBusy.value = true;
  try {
    const result = await api<{ id: string }>("/api/peers", {
      address: manualAddress.value.trim(),
    });
    modal.value = "";
    manualAddress.value = "";
    await refresh();
    const device = state.value?.devices.find((d) => d.id === result.id);
    if (device) await selectDevice(device);
  } catch (error) {
    notify((error as Error).message, true);
  } finally {
    modalBusy.value = false;
  }
}
function newGroup(edit = false) {
  addMenu.value = false;
  editingGroup.value = edit ? group.value?.id || "" : "";
  groupName.value = edit ? group.value?.name || "" : "";
  memberIds.value = [];
  modal.value = "group";
}
async function saveGroup() {
  modalBusy.value = true;
  try {
    if (editingGroup.value) {
      await api(`/api/groups/${editingGroup.value}/invite`, {
        name: groupName.value,
        members: memberIds.value,
      });
      modal.value = "";
      await refresh();
    } else {
      const result = await api<{ group: Group; unreachable: string[] }>(
        "/api/groups",
        { name: groupName.value, members: memberIds.value },
      );
      modal.value = "";
      await refresh();
      const c = state.value?.conversations.find(
        (c) => c.group_id === result.group.id,
      );
      if (c) await selectConversation(c);
      if (result.unreachable.length)
        notify(
          `群已创建，${result.unreachable.length} 位成员暂未收到邀请，可在群设置中重新发送`,
        );
    }
  } catch (error) {
    notify((error as Error).message, true);
  } finally {
    modalBusy.value = false;
  }
}
async function groupDecision(accept: boolean) {
  if (!group.value) return;
  modalBusy.value = true;
  try {
    await api(`/api/groups/${group.value.id}/decision`, { accept });
    await refresh();
  } catch (error) {
    notify((error as Error).message, true);
  } finally {
    modalBusy.value = false;
  }
}
async function resendInvitations() {
  if (!group.value) return;
  modalBusy.value = true;
  try {
    const value = await api<{ unreachable: string[] }>(
      `/api/groups/${group.value.id}/invite`,
      { members: [], name: group.value.name },
    );
    await refresh();
    notify(
      value.unreachable.length
        ? `${value.unreachable.length} 位成员暂不可达，其余邀请已发送`
        : "邀请已重新发送",
    );
  } catch (error) {
    notify((error as Error).message, true);
  } finally {
    modalBusy.value = false;
  }
}
async function leaveGroup() {
  if (!group.value) return;
  modalBusy.value = true;
  try {
    await api(`/api/groups/${group.value.id}/leave`, {});
    modal.value = "";
    await refresh();
  } catch (error) {
    notify((error as Error).message, true);
  } finally {
    modalBusy.value = false;
  }
}
async function saveName() {
  nameBusy.value = true;
  try {
    const saved = await api<{ name: string }>("/api/settings", {
      name: newName.value,
    });
    newName.value = saved.name;
    if (!state.value?.me.owner) {
      rememberBrowserName(saved.name);
      browserName.value = saved.name;
    }
    await refresh();
    notify("设备名称已保存");
  } catch (error) {
    notify((error as Error).message, true);
  } finally {
    nameBusy.value = false;
  }
}
async function clearFiles() {
  modalBusy.value = true;
  try {
    const result = await api<{ freed: number }>("/api/storage/clear", {});
    modal.value = "";
    releaseThumbnails();
    await refresh();
    notify(`已清理 ${size(result.freed)}，历史记录已保留`);
  } catch (error) {
    notify((error as Error).message, true);
  } finally {
    modalBusy.value = false;
  }
}
async function openProject() {
  const url = "https://github.com/whoarei/linchuan";
  try {
    if (native) {
      const { openUrl } = await import("@tauri-apps/plugin-opener");
      await openUrl(url);
    } else {
      window.open(url, "_blank", "noopener,noreferrer");
    }
  } catch (error) {
    notify((error as Error).message, true);
  }
}
async function disconnect() {
  try {
    await api("/api/session/logout", {});
  } catch {}
  forget();
  sessionChanged();
}
function sessionChanged() {
  clearTimeout(draftTimer);
  clearTimeout(eventTimer);
  socket?.close();
  socket = undefined;
  controller?.abort();
  selected.value = "";
  closeModal();
  code.value = "";
  showTemporaryHistory.value = undefined;
  messages.value = [];
  text.value = "";
  files.value = [];
  fileCache.clear();
  autoDownloads.clear();
  releaseThumbnails();
  closePreview();
  state.value = undefined;
  ready.value = false;
  mode.value = "devices";
  if (token())
    void refresh().then(() => {
      if (ready.value) events();
    });
}
function setMode(value: Mode) {
  flushDraft();
  dragging.value = false;
  mode.value = value;
  if (value === "settings") {
    newName.value = state.value?.me.name || "";
    void loadDesktopSettings();
  }
}
async function start() {
  loading.value = true;
  bootError.value = "";
  clearInterval(poll);
  try {
    await bootstrap();
    if (disposed) return;
    void loadDesktopSettings();
    void loadReceiveSettings();
    const info = await api<{ name: string }>("/api/info");
    if (disposed) return;
    hostName.value = info.name;
    if (token()) await refresh(true);
    if (disposed) return;
    if (ready.value) events();
    poll = setInterval(() => {
      if (token()) {
        void refresh();
        if (ready.value) events();
      }
    }, 3000);
  } catch (error) {
    const reason = error instanceof Error ? error.message : typeof error === "string" ? error : "";
    if (!disposed) bootError.value = reason || "请检查应用和网络后重试";
  } finally {
    if (!disposed) loading.value = false;
  }
}
onMounted(async () => {
  window.addEventListener("linchuan-session-change", sessionChanged);
  void start();
  if (!native) return;
  const listeners = [
    listen<NativeSelection[]>("linchuan-files", (event) => {
      if (mode.value !== "scan") nativeSelections(event.payload);
    }),
    listen<string>("linchuan-file-error", (event) => notify(event.payload, true)),
    listen<boolean>("linchuan-dragging", (event) => {
      dragging.value = mode.value !== "scan" && event.payload && canWrite.value;
    }),
  ];
  await Promise.all(listeners.map(async (listener) => {
    try {
      const unlisten = await listener;
      if (disposed) unlisten();
      else nativeListeners.push(unlisten);
    } catch { /* Test hosts may omit the event bridge. */ }
  }));
});
onUnmounted(() => {
  disposed = true;
  closeModal();
  nativeListeners.forEach((unlisten)=>unlisten());
  window.removeEventListener("linchuan-session-change", sessionChanged);
  clearInterval(poll);
  clearTimeout(draftTimer);
  clearTimeout(eventTimer);
  clearTimeout(toastTimer);
  socket?.close();
  controller?.abort();
  releaseThumbnails();
  closePreview();
});
</script>

<template>
  <DesktopTitleBar @error="notify($event, true)" />
  <ScreenAssistance
    v-if="ready && (native || (browserOnly && state?.info.capabilities?.includes('screen-share-v1')))"
    :browser="!native"
    :android="native && state?.info.platform === 'Android'"
    :linux="native && state?.info.platform === 'Linux'"
    ref="screenAssistance"
    @error="notify($event, true)"
    @ended="notify($event)"
  />
  <div v-if="loading || bootError" class="launch">
    <BrandMark />
    <h1>邻传</h1>
    <template v-if="bootError">
      <p class="error" role="alert">{{ bootError }}</p>
      <button class="secondary" @click="start">重试</button>
    </template>
    <LoaderCircle v-else class="spin" :size="22" />
  </div>
  <main v-else-if="!ready" class="pair-page">
    <section class="pair-card">
      <BrandMark />
      <h1>连接设备</h1>
      <p>配对 {{ hostName }}，开始传送文字和文件。</p>
      <form @submit.prevent="pair">
        <label
          >你的设备名称<input
            v-model="browserName"
            maxlength="64"
            :placeholder="browserIdentityName.automatic"
            autocomplete="nickname" /></label
        ><label
          >六位配对码<input
            v-model="code"
            required
            pattern="[0-9]{6}"
            inputmode="numeric"
            maxlength="6"
            placeholder="查看对方应用中的配对码"
        /></label>
        <p v-if="pairError" class="error" role="alert">{{ pairError }}</p>
        <button class="primary wide" :disabled="pairing">
          <LoaderCircle v-if="pairing" class="spin" :size="17" /><Link
            v-else
            :size="17"
          />{{ pairing ? "正在连接" : "连接设备" }}
        </button>
      </form>
      <small>同一局域网 · 无需账号 · HTTP 传输</small>
      <div v-if="!native" class="client-download">
        <a :href="downloadsUrl" target="_blank" rel="noopener noreferrer">
          <Download :size="15" />下载客户端<ArrowUpRight :size="13" />
        </a>
        <small>Android · Linux · Windows · GitHub</small>
      </div>
    </section>
  </main>
  <div
    v-else
    class="workspace"
    :inert="!!modal"
    :class="{
      'chat-open': !!selected && mode === 'devices',
      'browser-only': browserOnly,
      'section-open': mode !== 'devices',
    }"
  >
    <nav class="function-rail" aria-label="主要功能">
      <button class="rail-brand" aria-label="邻传" title="邻传" @click="setMode('devices')">
        <BrandMark />
      </button>
      <button
        class="rail-button"
        :class="{ active: mode === 'devices' }"
        :aria-label="browserOnly ? '收发' : '设备'"
        :title="browserOnly ? '收发' : '设备'"
        @click="setMode('devices')"
      >
        <MessageSquare :size="23" /><span>{{ browserOnly ? "收发" : "设备" }}</span
        ><i v-if="unread" class="rail-unread" />
      </button>
      <button
        v-if="native && state?.me.owner"
        class="rail-button"
        :class="{ active: mode === 'scan' }"
        :aria-current="mode === 'scan' ? 'page' : undefined"
        title="收发"
        @click="setMode('scan')"
      >
        <QrCode :size="23" /><span>收发</span>
      </button>
      <div class="rail-spacer" />
      <button
        class="rail-button"
        :class="{ active: mode === 'settings' }"
        aria-label="设置"
        title="设置"
        @click="setMode('settings')"
      >
        <Settings :size="22" /><span>设置</span></button
      ><button
        class="rail-button"
        :class="{ active: mode === 'about' }"
        aria-label="关于"
        title="关于"
        @click="setMode('about')"
      >
        <Info :size="22" /><span>关于</span>
      </button>
    </nav>
    <aside v-if="mode === 'devices' && !browserOnly" class="conversation-panel">
      <header class="list-header">
        <div>
          <h1>设备</h1>
          <span
            class="connection-dot"
            :class="{ offline: !connected }"
            :title="connected ? '局域网已连接' : '连接中断'"
          />
        </div>
        <div class="add-wrapper">
          <button
            class="icon-button"
            aria-label="添加"
            @click="addMenu = !addMenu"
          >
            <Plus :size="20" />
          </button>
          <div v-if="addMenu" class="popup-menu">
            <button
              @click="
                addMenu = false;
                modal = 'manual';
              "
            >
              <Link :size="16" />通过地址连接</button
            ><button @click="newGroup()"><Users :size="16" />新建群聊</button
            ><button v-if="state!.me.owner" @click="showConnection">
              <QrCode :size="16" />连接浏览器
            </button>
          </div>
        </div>
      </header>
      <div class="search-field">
        <Search :size="15" /><input
          v-model="search"
          aria-label="搜索设备或群聊"
          placeholder="搜索设备或群聊"
        />
      </div>
      <div class="conversation-scroll">
        <div v-if="conversations.length" class="list-label">会话</div>
        <ConversationRow
          v-for="conversation in conversations"
          :key="conversation.id"
          :conversation="conversation"
          :selected="conversation.id === selected"
          :label="conversationLabel(conversation)"
          :online="online(conversation.peer_id)"
          @select="selectConversation(conversation)"
        />
        <div v-if="nearby.length" class="list-label nearby-label">
          附近设备 <span>{{ nearby.length }}</span>
        </div>
        <button
          v-for="device in nearby"
          :key="device.id"
          class="conversation-row"
          :aria-label="`连接 ${deviceLabel(device.id, device.name)}`"
          @click="selectDevice(device)"
        >
          <span class="avatar"
            ><Smartphone
              v-if="device.platform === 'Android'"
              :size="23" /><Globe
              v-else-if="device.kind === 'browser'"
              :size="23" /><Monitor v-else :size="23" /><i /></span
          ><span class="row-copy"
            ><span class="row-title"
              ><b>{{ deviceLabel(device.id, device.name) }}</b></span
            ><span class="row-preview">{{ device.platform }} · 在线</span></span
          >
        </button>
        <section
          v-if="temporaryHistory.length"
          class="temporary-history"
          aria-label="临时会话记录"
        >
          <button
            class="history-toggle"
            :aria-expanded="temporaryHistoryExpanded"
            aria-controls="temporary-history-list"
            @click="showTemporaryHistory = !temporaryHistoryExpanded"
          >
            <ChevronDown
              v-if="temporaryHistoryExpanded"
              :size="14"
            /><ChevronRight v-else :size="14" />临时会话记录
            <span>{{ temporaryHistory.length }}</span><em
              v-if="historyUnread"
              >{{ historyUnread > 99 ? "99+" : historyUnread }}</em
            >
          </button>
          <div
            v-if="temporaryHistoryExpanded"
            id="temporary-history-list"
          >
            <ConversationRow
              v-for="conversation in temporaryHistory"
              :key="conversation.id"
              :conversation="conversation"
              :label="conversationLabel(conversation)"
              :selected="conversation.id === selected"
              :online="online(conversation.peer_id)"
              @select="selectConversation(conversation)"
            />
          </div>
        </section>
        <div
          v-if="
            !conversations.length && !nearby.length && !temporaryHistory.length
          "
          class="list-empty"
        >
          <Wifi :size="28" />
          <p>{{ search ? "没有找到相关设备" : "等待附近设备加入" }}</p>
          <span v-if="!search"
            >打开另一台设备上的邻传，<br />也可以通过地址连接。</span
          ><button v-if="!search" class="text-button" @click="modal = 'manual'">
            通过地址连接
          </button>
        </div>
      </div>
      <footer class="list-footer">
        <span class="self-avatar">{{ state!.me.name.slice(0, 1) }}</span><span
          ><b>{{ deviceLabel(state!.me.id, state!.me.name) }}</b
          ><small>{{
            state!.me.platform + (state!.me.owner ? " · 本机设备" : "")
          }}</small></span
        ><button
          v-if="state!.me.owner"
          class="icon-button"
          title="连接其他设备"
          aria-label="连接其他设备"
          @click="showConnection"
        >
          <QrCode :size="18" />
        </button>
      </footer>
    </aside>
    <main v-if="mode === 'devices'" class="chat-panel">
      <template v-if="current"
        ><header class="chat-header">
          <button
            v-if="!browserOnly"
            class="icon-button mobile-back"
            aria-label="返回设备列表"
            @click="
              flushDraft();
              selected = '';
            "
          >
            <ArrowLeft :size="21" />
          </button>
          <div class="chat-title">
            <h2>{{ conversationLabel(current) }}</h2>
            <p v-if="current.kind === 'group'">
              {{
                group?.members.filter((m) => m.state === "joined").length || 0
              }}
              位成员<template v-if="membership?.state === 'left'">
                · 已退出</template
              >
            </p>
            <p v-else>
              <span
                class="connection-dot"
                :class="{ offline: !online(current.peer_id) }"
                :title="browserOnly ? (connected ? '局域网已连接' : '连接中断') : (online(current.peer_id) ? '设备在线' : '设备离线')"
              /><template v-if="currentDevice"
                >{{ currentDevice.platform }} · </template
              >{{ online(current.peer_id) ? "在线" : "离线 · 历史记录保留" }}
            </p>
          </div>
          <button
            v-if="group"
            class="icon-button"
            aria-label="群成员与设置"
            @click="modal = 'members'"
          >
            <MoreHorizontal :size="23" /></button
          >
          <button
            v-else-if="
              screenAssistance?.canCapture && currentDevice &&
              (currentDevice.capabilities.includes('screen-share-v1') ||
                currentDevice.capabilities.includes('screen-view-web-v1'))
            "
            type="button"
            class="secondary"
            @click="screenAssistance?.start(currentDevice)"
          ><MonitorUp :size="16" />分享屏幕</button>
          <span v-else class="local-note">局域网直传</span>
          <button
            v-if="browserOnly"
            class="icon-button mobile-disconnect"
            aria-label="断开当前会话"
            title="断开当前会话"
            @click="disconnect"
          >
            <LogOut :size="20" />
          </button>
        </header>
        <div
          v-if="group && membership?.state === 'invited'"
          class="invitation-banner"
        >
          <Users :size="19" /><span
            ><b>你收到一个群邀请</b
            ><small>加入后可与群内成员直接传输。</small></span
          ><button
            class="secondary"
            :disabled="modalBusy"
            @click="groupDecision(false)"
          >
            拒绝</button
          ><button
            class="primary"
            :disabled="modalBusy"
            @click="groupDecision(true)"
          >
            加入群聊
          </button>
        </div>
        <div
          ref="timeline"
          class="message-timeline"
          @dragover.prevent="dragging = canWrite"
          @dragleave.prevent="dragging = false"
          @drop="drop"
        >
          <button
            v-if="before"
            class="load-history"
            :disabled="historyBusy"
            @click="loadMessages(false, true)"
          >
            <LoaderCircle
              v-if="historyBusy"
              class="spin"
              :size="14"
            />加载更早记录
          </button>
          <div v-if="!messages.length && !historyBusy" class="timeline-empty">
            <MessageSquare :size="30" />
            <p>
              {{
                membership?.state === "invited"
                  ? "等待你确认加入群聊"
                  : "还没有消息，发送第一条内容。"
              }}
            </p>
          </div>
          <article
            v-for="message in messages"
            :key="message.id"
            class="message"
            :class="{ outgoing: message.sender_id === state!.me.id }"
            :data-message-id="message.id"
          >
            <div class="message-avatar">
              {{ message.sender_name.slice(0, 1) }}
            </div>
            <div class="message-main">
              <div class="message-meta">
                <span>{{
                  message.sender_id === state!.me.id
                    ? "我"
                    : message.sender_name
                }}</span><time>{{ timestamp(message.created_at) }}</time>
              </div>
              <div v-if="message.text" class="text-bubble">
                <p>{{ message.text }}</p>
                <button
                  class="copy-message"
                  aria-label="复制文字"
                  @click="copy(message.text)"
                >
                  <Copy :size="14" />
                </button>
              </div>
              <div v-if="message.files.length" class="message-attachments">
                <div
                  v-if="incomingPending(message)"
                  class="attachment-actions attachment-receive-actions"
                  role="group"
                  aria-label="接收文件"
                >
                  <span>
                    待接收<template v-if="incomingPending(message)!.can_select_files">
                      · 已选 {{ selectedForReceive(incomingPending(message)!).length }}/{{ message.files.length }} 个文件
                    </template><template v-else-if="message.files.length > 1">
                      · {{ message.files.length }} 个文件，一起接收
                    </template>
                  </span>
                  <button
                    class="icon-button"
                    aria-label="拒绝"
                    title="拒绝"
                    :disabled="decisionBusy.size > 0"
                    @click="decide(incomingPending(message)!, 'reject')"
                  >
                    <LoaderCircle
                      v-if="decisionBusy.get(incomingPending(message)!.id) === 'reject'"
                      class="spin"
                      :size="18"
                    /><X v-else :size="18" />
                  </button>
                  <button
                    v-if="native"
                    class="icon-button"
                    aria-label="另存为"
                    title="另存为"
                    :disabled="decisionBusy.size > 0 || !selectedForReceive(incomingPending(message)!).length"
                    @click="decide(incomingPending(message)!, 'save-as')"
                  >
                    <FolderInput :size="18" aria-hidden="true" />
                  </button>
                  <button
                    class="icon-button"
                    aria-label="接收"
                    title="接收"
                    :disabled="decisionBusy.size > 0 || !selectedForReceive(incomingPending(message)!).length"
                    @click="decide(incomingPending(message)!, 'accept')"
                  >
                    <LoaderCircle
                      v-if="decisionBusy.get(incomingPending(message)!.id) === 'accept'"
                      class="spin"
                      :size="18"
                    /><Check v-else :size="18" />
                  </button>
                </div>
                <div
                  v-else-if="incomingActive(message)"
                  class="attachment-transfer"
                  role="status"
                >
                  <div>
                    <LoaderCircle class="spin" :size="13" />
                    <span>正在接收…</span>
                    <small>{{ transferProgress(incomingActive(message)!) }}</small>
                    <button
                      class="text-button"
                      @click="cancel(incomingActive(message)!)"
                    >
                      取消
                    </button>
                  </div>
                  <div v-if="browserOnly" class="browser-downloads">
                    <button v-for="file in message.files.filter((file) => includesFile(incomingActive(message), file.id))" :key="file.id" class="text-button" @click="download(file, message.id)">下载 {{ file.name }}</button>
                    <small>文件由浏览器下载管理器保存。</small>
                  </div>
                  <progress
                    aria-label="文件接收进度"
                    :value="incomingActive(message)!.bytes"
                    :max="incomingActive(message)!.total || 1"
                  />
                </div>
                <div
                  v-for="file in message.files"
                  :key="file.id"
                  class="attachment-card"
                  :class="{
                    expired:
                      !file.available &&
                      includesFile(incomingDelivery(message), file.id) &&
                      receipt(message.id, file.id)?.status !== 'saved' &&
                      message.deliveries.every((d) => terminal(d.status)),
                  }"
                >
                  <div
                    v-if="
                      !incomingPending(message) &&
                      ((file.available && !incomingActive(message)) ||
                        (native &&
                          receipt(message.id, file.id)?.status === 'saved'))
                    "
                    class="attachment-actions"
                    role="group"
                    :aria-label="`${file.name} 的文件操作`"
                  >
                    <button v-if="native && file.original" class="icon-button" :aria-label="`打开原文件 ${file.name}`" title="打开原文件" @click="openSavedFile(message.id,file.id)"><ArrowUpRight :size="18" /></button>
                    <LoaderCircle
                      v-if="fileBusy.has(`${message.id}:${file.id}`)"
                      class="spin"
                      :size="16"
                      aria-label="正在处理文件"
                    />
                    <button
                      v-if="
                        file.available &&
                        raster(file) &&
                        !thumbnails[message.id + file.id]
                      "
                      class="icon-button"
                      :aria-label="`预览 ${file.name}`"
                      title="预览图片"
                      @click="viewImage(file)"
                    >
                      <ImageIcon :size="17" />
                    </button>
                    <button
                      v-if="
                        file.available &&
                        (!native ||
                          ['failed', 'replaced'].includes(receipt(message.id, file.id)?.status || ''))
                      "
                      class="icon-button"
                      :disabled="
                        receipt(message.id, file.id)?.status === 'pending'
                      "
                      :aria-label="`${native ? '重新保存' : '下载'} ${file.name}`"
                      :title="native ? '重新保存到接收目录' : '下载'"
                      @click="download(file, message.id)"
                    >
                      <Download :size="18" />
                    </button>
                    <button
                      v-if="
                        native &&
                        (file.available ||
                          receipt(message.id, file.id)?.status === 'saved')
                      "
                      class="icon-button"
                      :disabled="
                        fileBusy.has(`${message.id}:${file.id}`) ||
                        receipt(message.id, file.id)?.status === 'pending'
                      "
                      :aria-label="`复制到其他位置 ${file.name}`"
                      title="复制到其他位置"
                      @click="relocateFile(message.id, file)"
                    >
                      <Copy :size="18" />
                    </button>
                    <template
                      v-if="
                        native &&
                        receipt(message.id, file.id)?.status === 'saved'
                      "
                    >
                      <button
                        class="icon-button"
                        :disabled="fileBusy.has(`${message.id}:${file.id}`)"
                        :aria-label="`移动到其他位置 ${file.name}`"
                        title="移动到其他位置"
                        @click="relocateFile(message.id, file, true)"
                      >
                        <FolderInput :size="18" />
                      </button>
                      <button
                        class="icon-button"
                        :aria-label="`打开所在文件夹 ${file.name}`"
                        title="打开所在文件夹"
                        @click="openSavedFile(message.id, file.id, true)"
                      >
                        <FolderOpen :size="18" />
                      </button>
                      <button
                        class="icon-button"
                        :aria-label="`打开文件 ${file.name}`"
                        title="打开文件"
                        @click="openSavedFile(message.id, file.id)"
                      >
                        <ArrowUpRight :size="18" />
                      </button>
                    </template>
                  </div>
                  <button
                    v-if="thumbnails[message.id + file.id]"
                    class="image-thumbnail"
                    :aria-label="`预览 ${file.name}`"
                    @click="viewImage(file)"
                  >
                    <img
                      :src="thumbnails[message.id + file.id]"
                      :alt="file.name"
                    />
                  </button>
                  <div class="attachment-body">
                    <input
                      v-if="incomingPending(message)?.can_select_files"
                      type="checkbox"
                      class="attachment-checkbox"
                      :aria-label="`接收 ${file.name}`"
                      :checked="!excludedFiles.get(incomingPending(message)!.id)?.has(file.id)"
                      :disabled="decisionBusy.size > 0"
                      @change="toggleReceiveFile(incomingPending(message)!, file.id, $event)"
                    />
                    <span class="file-icon"
                      ><ImageIcon
                        v-if="file.mime.startsWith('image/')"
                        :size="24" /><File v-else :size="24" /></span
                    ><span class="file-label"
                      ><b>{{ receipt(message.id, file.id)?.status === "saved" ? receipt(message.id, file.id)?.name : file.name }}</b
                      ><small
                        :title="
                          receipt(message.id, file.id)?.status === 'saved'
                            ? receipt(message.id, file.id)?.location
                            : undefined
                        "
                        >{{ size(file.size) }}
                        <template v-if="!includesFile(incomingDelivery(message), file.id)">
                          · 未选择接收</template
                        >
                        <template
                          v-else-if="
                            receipt(message.id, file.id)?.status === 'saved'
                          "
                        >
                          · 已保存</template
                        >
                        <template v-else-if="receipt(message.id, file.id)?.status === 'replaced'"> · 原文件已被覆盖</template>
                        <template
                          v-else-if="
                            receipt(message.id, file.id)?.status === 'pending'
                          "
                        >
                          · 正在保存…</template
                        >
                        <template
                          v-else-if="
                            receipt(message.id, file.id)?.status === 'failed'
                          "
                        >
                          · 保存失败</template
                        >
                        <template
                          v-else-if="
                            incomingDelivery(message)?.status === 'completed' &&
                            file.available
                          "
                        >
                          · 已接收</template
                        >
                        <template v-else-if="incomingDelivery(message) && ['rejected', 'cancelled', 'failed', 'interrupted'].includes(incomingDelivery(message)!.status)">
                          · {{ statusLabel(incomingDelivery(message)!.status) }}</template
                        >
                        <template v-else-if="!file.available && message.deliveries.every((d) => terminal(d.status))">
                          · 文件不可用或已过期</template
                        >
                      </small></span
                    >
                  </div>
                  <p
                    v-if="receipt(message.id, file.id)?.status === 'failed'"
                    class="save-error"
                    role="status"
                  >
                    {{
                      receipt(message.id, file.id)?.error
                    }}。可重试或在设置中更改接收目录。
                  </p>
                </div>
              </div>
              <div
                v-if="!message.files.length && incomingPending(message)"
                class="receive-actions"
              >
                <span>是否接收这条文字？</span
                ><button
                  class="secondary"
                  :disabled="decisionBusy.size > 0"
                  @click="decide(incomingPending(message)!, 'reject')"
                >
                  拒绝</button
                ><button
                  class="primary"
                  :disabled="decisionBusy.size > 0"
                  @click="decide(incomingPending(message)!, 'accept')"
                >
                  接收
                </button>
              </div>
              <button
                v-if="message.sender_id === state!.me.id"
                class="delivery-summary"
                :class="{ success: summary(message) === '已送达' }"
                @click="toggleExpanded(message.id)"
              >
                <CheckCheck
                  v-if="summary(message) === '已送达'"
                  :size="13"
                /><LoaderCircle
                  v-else-if="summary(message) === '正在传输'"
                  class="spin"
                  :size="13"
                />{{ summary(message)
                }}<ChevronDown v-if="message.deliveries.length" :size="12" />
              </button>
              <div
                v-if="
                  visibleDeliveries(message).length &&
                  (expanded.has(message.id) ||
                    visibleDeliveries(message).some(
                      (d) => d.status === 'transferring' || d.error,
                    ))
                "
                class="delivery-details"
              >
                <div
                  v-for="delivery in visibleDeliveries(message)"
                  :key="delivery.id"
                >
                  <span>{{
                    delivery.recipient_id === state!.me.id
                      ? "本机"
                      : delivery.recipient_name
                  }}</span
                  ><span>{{ statusLabel(delivery.status) }}<template v-if="delivery.status === 'completed' && delivery.selected_file_ids">
                    · {{ delivery.selected_file_ids.length }}/{{ delivery.files.length }} 个文件
                  </template></span>
                  <small v-if="delivery.status === 'transferring'">{{ transferProgress(delivery) }}</small>
                  <button
                    v-if="!terminal(delivery.status)"
                    class="text-button"
                    @click="cancel(delivery)"
                  >
                    取消</button
                  ><progress
                    v-if="delivery.status === 'transferring'"
                    :value="delivery.bytes"
                    :max="delivery.total || 1"
                  />
                  <p v-if="delivery.error">{{ delivery.error }}</p>
                </div>
              </div>
            </div>
          </article>
          <div v-if="dragging" class="drop-overlay">
            <Paperclip :size="29" /><span>松开以添加文件</span>
          </div>
        </div>
        <div class="composer" :class="{ disabled: !canWrite }">
          <div v-if="files.length" class="composer-files">
            <div v-for="(file, index) in files" :key="index">
              <File :size="17" /><span>{{ file.name }}</span
              ><small>{{ size(file.size) }}</small
              ><button
                :disabled="sending"
                class="icon-button"
                :aria-label="`移除 ${file.name}`"
                @click="files.splice(index, 1)"
              >
                <X :size="14" />
              </button>
            </div>
          </div>
          <div class="composer-toolbar">
            <button
              class="icon-button"
              title="添加文件或图片"
              aria-label="添加文件或图片"
              :disabled="!canWrite || sending"
              @click="chooseFiles()"
            >
              <Paperclip :size="20" /></button
            ><span>{{
              sending ? sendPhase : "可粘贴截图或拖入文件"
            }}</span
            ><span v-if="sending" class="upload-count"
              >{{ size(progress) }} / {{ size(uploadTotal)
              }}<button class="text-button" @click="controller?.abort()">
                取消
              </button></span
            ><input
              ref="input"
              type="file"
              multiple
              hidden
              :disabled="!canWrite || sending"
              @change="fileChange"
            />
          </div>
          <progress
            v-if="sending && uploadTotal"
            :value="progress"
            :max="uploadTotal"
          /><textarea
            ref="composer"
            v-model="text"
            aria-label="发送文字"
            :aria-invalid="textTooLong"
            :aria-describedby="textTooLong ? 'text-limit-error' : undefined"
            :disabled="!canWrite || sending"
            :placeholder="
              canWrite
                ? '输入文字…'
                : membership?.state === 'invited'
                  ? '加入群聊后即可发送'
                  : '已退出群聊，可继续查看本机历史'
            "
            @input="queueDraft"
            @paste="paste"
            @keydown.ctrl.enter.prevent="send"
            @keydown.meta.enter.prevent="send"
          />
          <p v-if="textTooLong" id="text-limit-error" class="error text-limit-error" role="alert">
            文字超过 65,536 字节，请缩短后再发送。文件尚未上传。
          </p>
          <div class="composer-footer">
            <small>Ctrl / ⌘ + Enter 发送</small
            ><button
              class="primary send-button"
              :disabled="!canSend"
              @click="send"
            >
              <LoaderCircle v-if="sending" class="spin" :size="15" /><Send
                v-else
                :size="15"
              />{{ sending ? (sendPhase.startsWith("等待") ? "等待接收" : "发送中") : "发送" }}
            </button>
          </div>
        </div>
      </template>
      <div v-else class="welcome">
        <BrandMark />
        <h2>邻传</h2>
        <p>选择设备或群聊，开始传输。</p>
        <button class="secondary" @click="modal = 'manual'">
          <Link :size="16" />通过地址连接
        </button>
      </div>
    </main>
    <ScanPublisher
      v-else-if="mode === 'scan' && native && state?.me.owner"
      :addresses="state.info.addresses.length ? state.info.addresses : [address()]"
      @copy="copy"
    />
    <main v-else-if="mode === 'settings'" class="settings-page">
      <header class="section-header">
        <h1>设置</h1>
        <p>管理本机设备与本地存储</p>
      </header>
      <section class="settings-card">
        <h2>设备名称</h2>
        <p>自动生成动物昵称，也可以修改。附近设备将通过这个名字识别你。</p>
        <form class="name-form" @submit.prevent="saveName">
          <label class="sr-only" for="device-name">设备名称</label
          ><input
            id="device-name"
            v-model="newName"
            required
            maxlength="64"
          /><button class="primary" :disabled="nameBusy">
            <LoaderCircle
              v-if="nameBusy"
              class="spin"
              :size="15"
            />保存名称</button
          ><button
            type="button"
            class="secondary"
            :disabled="nameBusy"
            @click="shuffleName"
          >
            换个名字
          </button>
        </form>
      </section>
      <section
        v-if="desktopSettings?.supported"
        class="settings-card"
        aria-label="启动与后台"
      >
        <h2>启动与后台</h2>
        <label class="background-setting">
          <span
            ><b>开机启动</b
            ><small>登录系统后自动启动，在托盘后台运行。</small></span
          >
          <input
            type="checkbox"
            role="switch"
            aria-label="开机启动"
            :checked="desktopSettings.autostart"
            :disabled="desktopBusy"
            @change="setAutostart(($event.target as HTMLInputElement).checked)"
          />
        </label>
        <label class="background-setting">
          <span
            ><b>关闭窗口后继续运行</b
            ><small>保持接收文件，可从托盘打开窗口或退出。</small></span
          >
          <input
            type="checkbox"
            role="switch"
            aria-label="关闭窗口后继续运行"
            :checked="desktopSettings.close_to_tray"
            :disabled="desktopBusy || !desktopSettings.tray_available"
            @change="
              setCloseToTray(($event.target as HTMLInputElement).checked)
            "
          />
        </label>
        <p v-if="!desktopSettings.tray_available" class="background-fallback">
          当前托盘不可用，关闭窗口会退出应用。
        </p>
        <button class="secondary" @click="quitDesktop">
          <LogOut :size="16" />退出邻传
        </button>
      </section>
      <section class="settings-card" aria-label="接收目录">
        <h2>接收目录</h2>
        <template v-if="native">
          <p>接收完成的文件和图片自动保存到此处。</p>
          <div class="receive-path">
            {{ receiveSettings?.label || "正在读取…" }}
          </div>
          <div class="receive-directory-actions">
            <button
              class="secondary"
              :disabled="receiveBusy"
              @click="changeDirectory()"
            >
              <FolderOpen :size="16" />更改目录
            </button>
            <button
              class="secondary"
              :disabled="receiveBusy"
              @click="openDirectory"
            >
              打开目录
            </button>
            <button
              class="text-button"
              :disabled="receiveBusy"
              @click="changeDirectory(true)"
            >
              恢复默认
            </button>
          </div>
        </template>
        <p v-else>
          文件保存到浏览器设置的下载位置；可在浏览器设置中更改目录或关闭逐次询问。
        </p>
      </section>
      <section v-if="state!.me.owner" class="settings-card">
        <div class="card-heading">
          <div>
            <h2>连接其他设备</h2>
            <p>让未安装应用的设备通过浏览器加入。</p>
          </div>
          <button class="secondary" @click="showConnection">
            <QrCode :size="17" />二维码与配对码
          </button>
        </div>
      </section>
      <section class="settings-card">
        <div class="card-heading">
          <div>
            <h2>本地存储</h2>
            <p>应用文件与任务占用 {{ size(state!.storage_used) }}</p>
            <p>
              {{
                state!.storage_available == null
                  ? "暂时无法读取磁盘剩余空间"
                  : `当前还可分配 ${size(state!.storage_available)}`
              }}
              · 默认预留 2 GiB
            </p>
          </div>
          <button
            v-if="state!.me.owner"
            class="secondary"
            @click="modal = 'clear'"
          >
            <Trash2 :size="16" />清理文件缓存
          </button>
        </div>
        <small
          >文字和历史记录保存在本机。清理缓存不会删除接收目录中的文件；待保存或保存失败的副本暂不清理。</small
        >
      </section>
      <section v-if="!state!.me.owner" class="settings-card">
        <button class="secondary" @click="disconnect">
          <LogOut :size="16" />断开连接
        </button>
      </section>
    </main>
    <main v-else class="about-page">
      <BrandMark />
      <h1>邻传 · Linchuan</h1>
      <span class="version-label">{{ state!.info.version }}</span>
      <section v-if="native" class="app-updates" aria-label="应用更新">
        <div class="update-heading">
          <b>版本更新</b>
          <button
            class="secondary"
            :disabled="updateBusy || updateState.phase === 'installed'"
            @click="checkForUpdate"
          >
            <RefreshCw
              :size="14"
              :class="{ spin: updateState.phase === 'checking' }"
            />检查更新
          </button>
        </div>
        <p role="status" aria-live="polite">{{ updateMessage }}</p>
        <p v-if="updateState.error" class="error" role="alert">
          {{ updateState.error }}
        </p>
        <progress
          v-if="updateState.phase === 'downloading'"
          :max="updateState.total || 1"
          :value="updateState.total ? updateState.downloaded : undefined"
          aria-label="更新下载进度"
        />
        <details v-if="updateState.info?.available && updateState.info.notes">
          <summary>更新内容</summary>
          <pre>{{ updateState.info.notes }}</pre>
        </details>
        <div v-if="updateState.info?.available" class="update-actions">
          <button
            v-if="updateState.phase === 'available' && updateState.inApp"
            class="primary"
            @click="downloadUpdate"
          >
            <Download :size="15" />下载更新
          </button>
          <button
            v-else-if="updateState.phase === 'available'"
            class="primary"
            @click="openUpdatePage(true)"
          >
            <Download :size="15" />下载新版安装包
          </button>
          <button
            v-if="updateState.phase === 'ready'"
            class="primary"
            @click="installUpdate"
          >
            安装并重启
          </button>
          <button
            v-if="updateState.phase === 'installed'"
            class="primary"
            @click="restartAfterUpdate"
          >
            重启应用
          </button>
          <button
            v-if="!updateBusy"
            class="text-button"
            @click="openUpdatePage(false)"
          >
            查看发布页<ArrowUpRight :size="13" />
          </button>
        </div>
      </section>
      <p>同一网络，随手互传。</p>
      <div class="about-facts">
        <div>
          <Monitor :size="18" /><span>Android、Linux、Windows 与浏览器</span>
        </div>
        <div>
          <Users :size="18" /><span>设备直传与群聊，历史保存在本机</span>
        </div>
        <div><ShieldCheck :size="18" /><span>无需账号，无云端中转</span></div>
      </div>
      <p class="about-note">
        仅在可信局域网使用，当前采用 HTTP 传输。<br />保持应用运行；群聊暂不自动补发离线消息。
      </p>
      <div class="about-links">
        <a
          v-if="!native"
          :href="downloadsUrl"
          target="_blank"
          rel="noopener noreferrer"
        >
          <Download :size="15" />下载客户端<ArrowUpRight :size="14" />
        </a>
        <a
          href="https://github.com/whoarei/linchuan"
          @click.prevent="openProject"
          target="_blank"
          rel="noopener noreferrer"
          >GitHub 项目<ArrowUpRight :size="14"
        /></a>
      </div>
    </main>
  </div>
  <div v-if="toast" class="toast" :class="{ error: toastError }" role="status">
    <X v-if="toastError" :size="17" /><Check v-else :size="17" />{{ toast }}
  </div>
  <div v-if="modal" class="modal-backdrop" @click.self="closeModal">
    <section
      ref="modalContainer"
      class="modal"
      :class="{ 'image-modal': modal === 'preview' }"
      role="dialog"
      aria-modal="true"
      aria-labelledby="modal-title"
      tabindex="-1"
    >
      <button
        class="modal-close icon-button"
        aria-label="关闭"
        @click="closeModal"
      >
        <X :size="20" />
      </button>
      <ReceiveConflictForm v-if="modal === 'receive-conflicts' && conflictPrompt"
        :conflicts="conflictPrompt.conflicts" :files="conflictPrompt.files" :can-skip="conflictPrompt.canSkip"
        :directory="conflictPrompt.directory" :format-size="size" @confirm="finishConflictPrompt" @cancel="closeModal" />
      <template v-else-if="modal === 'connect'"
        ><div class="modal-icon"><QrCode :size="24" /></div>
        <h2 id="modal-title">连接其他设备</h2>
        <p>在同一网络中，扫码或用浏览器打开地址。</p>
        <img :src="qr" class="qr-image" alt="浏览器访问二维码" /><select
          v-if="state!.info.addresses.length > 1 || browserHttps"
          v-model="shareAddress"
          aria-label="选择网络地址"
          @change="updateQr"
        >
          <option v-for="item in state!.info.addresses" :key="item">
            {{ item }}
          </option>
          <option v-if="browserHttps" :value="browserHttps.address">网页协助（HTTPS）</option></select
        ><button class="address-copy" @click="copy(shareAddress)">
          <span>{{ shareAddress }}</span><Copy :size="16" />
        </button>
        <div class="pairing-code">
          <small>{{ httpsAddressSelected ? "网页协助配对码" : "本次配对码" }}</small><b>{{ shareCode }}</b>
        </div>
        <button v-if="native && state?.info.capabilities?.includes('screen-share-v1')" class="secondary" type="button" @click="configureBrowserHttps">
          配置网页控制
        </button></template
      >
      <template v-else-if="modal === 'browser-https'">
        <h2 id="modal-title">配置网页控制</h2>
        <p>请使用观看方浏览器信任的证书。域名或 IP 地址需与证书匹配。</p>
        <form @submit.prevent="saveBrowserHttps">
          <label>域名或 IP 地址<input v-model="httpsHost" maxlength="253" required /></label>
          <label>HTTPS 端口<input v-model.number="httpsPort" type="number" min="1" max="65535" placeholder="留空自动分配" /></label>
          <label>PEM 证书链<input type="file" required @change="choosePem($event, 'certificate')" /></label>
          <label>PEM 私钥<input type="file" required @change="choosePem($event, 'key')" /></label>
          <div class="modal-actions">
            <button class="secondary" type="button" :disabled="modalBusy" @click="showConnection">返回</button>
            <button class="primary" :disabled="modalBusy">保存</button>
          </div>
        </form>
      </template>
      <template v-else-if="modal === 'manual'"
        ><div class="modal-icon"><Link :size="24" /></div>
        <h2 id="modal-title">通过地址连接</h2>
        <p>输入对方应用展示的完整访问地址。</p>
        <form @submit.prevent="connectPeer">
          <label
            >对方设备地址<input
              v-model="manualAddress"
              type="url"
              required
              placeholder="http://192.168.1.20:52783" /></label
          ><button class="primary wide" :disabled="modalBusy">
            <LoaderCircle v-if="modalBusy" class="spin" :size="16" />{{
              modalBusy ? "连接中" : "连接设备"
            }}
          </button>
        </form></template
      >
      <template v-else-if="modal === 'group'"
        ><div class="modal-icon"><Users :size="24" /></div>
        <h2 id="modal-title">{{ editingGroup ? "管理群聊" : "新建群聊" }}</h2>
        <p>
          {{
            editingGroup
              ? "修改群名，或邀请附近设备加入。"
              : "选择成员，对方确认加入后即可收发。"
          }}
        </p>
        <form @submit.prevent="saveGroup">
          <label
            >群名称<input
              v-model="groupName"
              required
              maxlength="64"
              placeholder="例如：家里的设备"
          /></label>
          <div class="member-selector">
            <label
              v-for="device in eligible"
              :key="device.id"
              class="member-option"
              ><input
                v-model="memberIds"
                type="checkbox"
                :value="device.id"
              /><span class="avatar small-avatar"><Monitor :size="18" /></span
              ><span
                >{{ deviceLabel(device.id, device.name)
                }}<small>{{ device.platform }}</small></span
              ></label
            >
            <p v-if="!eligible.length">暂无支持群聊的附近设备</p>
          </div>
          <button
            class="primary wide"
            :disabled="modalBusy || (!editingGroup && !memberIds.length)"
          >
            <LoaderCircle v-if="modalBusy" class="spin" :size="16" />{{
              editingGroup ? "保存并发送邀请" : "创建群聊"
            }}
          </button>
        </form></template
      >
      <template v-else-if="modal === 'members' && group"
        ><h2 id="modal-title">{{ group.name }}</h2>
        <p>
          {{ group.members.length }} 位成员 ·
          {{ group.members.filter((m) => m.state === "joined").length }}
          位已加入
        </p>
        <div class="member-list">
          <div v-for="member in group.members" :key="member.device.id">
            <span class="avatar small-avatar">{{
              member.device.name.slice(0, 1)
            }}</span><span
              >{{ deviceLabel(member.device.id, member.device.name)
              }}<small
                >{{ member.role === "owner" ? "群主 · " : ""
                }}{{
                  member.state === "joined"
                    ? "已加入"
                    : member.state === "invited"
                      ? "等待确认"
                      : "已退出"
                }}</small
              ></span
            >
          </div>
        </div>
        <button
          v-if="owner && membership?.state === 'joined'"
          class="secondary wide"
          @click="newGroup(true)"
        >
          <Users :size="16" />修改群名或邀请成员</button
        ><button
          v-if="owner && membership?.state === 'joined'"
          class="text-button wide"
          @click="resendInvitations"
          :disabled="modalBusy"
        >
          重新发送邀请</button
        ><button
          v-if="membership?.state === 'joined'"
          class="danger-button wide"
          @click="modal = 'leave'"
        >
          退出群聊
        </button></template
      >
      <template v-else-if="modal === 'leave'"
        ><h2 id="modal-title">退出群聊？</h2>
        <p>本机历史会保留。再次加入需要群主重新邀请。</p>
        <div class="modal-actions">
          <button class="secondary" @click="modal = 'members'">取消</button
          ><button class="primary" :disabled="modalBusy" @click="leaveGroup">
            确认退出
          </button>
        </div></template
      >
      <template v-else-if="modal === 'clear'"
        ><h2 id="modal-title">清理文件缓存？</h2>
        <p>
          文字和历史卡片会保留，正在传输的文件会跳过。已清理的附件需向发送者重新获取。
        </p>
        <div class="modal-actions">
          <button class="secondary" @click="closeModal">取消</button
          ><button class="primary" :disabled="modalBusy" @click="clearFiles">
            确认清理
          </button>
        </div></template
      >
      <template v-else-if="modal === 'preview' && preview"
        ><h3 id="modal-title">{{ preview.name }}</h3>
        <img :src="preview.url" :alt="preview.name" class="preview-image"
      /></template>
    </section>
  </div>
</template>
