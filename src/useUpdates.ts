import { computed, onUnmounted, reactive } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { Update } from "@tauri-apps/plugin-updater";

interface ReleaseInfo {
  current_version: string;
  version: string;
  available: boolean;
  notes: string;
  release_url: string;
  download_url: string;
  in_app: boolean;
}
type Phase =
  | "idle"
  | "checking"
  | "current"
  | "available"
  | "downloading"
  | "ready"
  | "installing"
  | "restarting"
  | "installed";

export function useUpdates() {
  const state = reactive({
    phase: "idle" as Phase,
    info: undefined as ReleaseInfo | undefined,
    error: "",
    downloaded: 0,
    total: 0,
    inApp: false,
  });
  let update: Update | null = null;
  let disposed = false;
  const busy = computed(() =>
    ["checking", "downloading", "installing", "restarting"].includes(
      state.phase,
    ),
  );
  const message = computed(() => {
    switch (state.phase) {
      case "checking":
        return "正在检查新版本…";
      case "current":
        return "当前版本无需更新";
      case "available":
        return `发现新版本 ${state.info?.version}`;
      case "downloading":
        return state.total
          ? `正在下载 ${Math.min(100, Math.round((state.downloaded / state.total) * 100))}%`
          : `已下载 ${(state.downloaded / 1024 / 1024).toFixed(1)} MB`;
      case "ready":
        return "更新包已下载并通过签名校验，安装后将重启应用";
      case "installing":
        return "正在安装更新，请稍候…";
      case "installed":
        return "更新已安装，重启应用后生效";
      case "restarting":
        return "更新已安装，正在重启应用…";
      default:
        return "手动检查并选择安装新版本";
    }
  });
  async function releaseResource(candidate: Update | null) {
    if (candidate) await candidate.close().catch(() => {});
  }
  async function checkForUpdate() {
    if (busy.value || state.phase === "installed") return;
    state.phase = "checking";
    state.error = "";
    state.inApp = false;
    state.info = undefined;
    await releaseResource(update);
    update = null;
    try {
      const info = await invoke<ReleaseInfo>("check_release");
      if (disposed) return;
      state.info = info;
      state.phase = info.available ? "available" : "current";
      if (info.available && info.in_app) {
        state.phase = "checking";
        const { check } = await import("@tauri-apps/plugin-updater");
        const candidate = await check({ timeout: 20000 });
        if (disposed) {
          await releaseResource(candidate);
          return;
        }
        if (!candidate || candidate.version !== info.version) {
          await releaseResource(candidate);
          throw new Error("在线更新信息暂不可用，可前往发布页下载新版");
        }
        update = candidate;
        state.inApp = true;
        state.phase = "available";
      }
    } catch (error) {
      state.phase = state.info?.available ? "available" : "idle";
      state.error = String(error);
    }
  }
  async function downloadUpdate() {
    if (!update || busy.value) return;
    const candidate = update;
    state.phase = "downloading";
    state.error = "";
    state.downloaded = state.total = 0;
    try {
      await candidate.download(
        (event) => {
          if (disposed) return;
          if (event.event === "Started")
            state.total = event.data.contentLength || 0;
          else if (event.event === "Progress")
            state.downloaded += event.data.chunkLength;
        },
        { timeout: 120000 },
      );
      if (!disposed) state.phase = "ready";
    } catch (error) {
      state.phase = "available";
      state.error = `下载或签名校验失败：${String(error)}`;
    } finally {
      if (disposed) await releaseResource(candidate);
    }
  }
  async function restartAfterUpdate() {
    if (state.phase !== "installed") return;
    state.phase = "restarting";
    state.error = "";
    try {
      const { relaunch } = await import("@tauri-apps/plugin-process");
      await relaunch();
    } catch (error) {
      state.phase = "installed";
      state.error = `请手动重启应用：${String(error)}`;
    }
  }
  async function installUpdate() {
    if (!update || state.phase !== "ready") return;
    state.phase = "installing";
    state.error = "";
    try {
      await invoke("prepare_update");
      await update.install();
      state.phase = "installed";
      await restartAfterUpdate();
    } catch (error) {
      state.phase = "ready";
      state.error = String(error);
    }
  }
  async function openUpdatePage(download = false) {
    if (!state.info) return;
    try {
      const { openUrl } = await import("@tauri-apps/plugin-opener");
      await openUrl(
        download ? state.info.download_url : state.info.release_url,
      );
    } catch (error) {
      state.error = String(error);
    }
  }
  onUnmounted(() => {
    disposed = true;
    void releaseResource(update);
  });
  return {
    state,
    busy,
    message,
    checkForUpdate,
    downloadUpdate,
    installUpdate,
    restartAfterUpdate,
    openUpdatePage,
  };
}
