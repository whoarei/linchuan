import { reactive, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { native } from "./api";
import type { MessageFile } from "./types";

interface ReceiveSettings {
  directory: string;
  label: string;
}
interface SavedFile {
  message_id: string;
  file_id: string;
  name: string;
  status: "pending" | "saved" | "failed";
  error: string | null;
}

export function useReceiving(
  report: (message: string, error?: boolean) => void,
) {
  const settings = ref<ReceiveSettings>();
  const busy = ref(false);
  const records = reactive(new Map<string, SavedFile>());
  const key = (message: string, file: string) => `${message}:${file}`;
  const receipt = (message: string, file: string) =>
    records.get(key(message, file));
  async function loadSettings() {
    if (!native) return;
    try {
      settings.value = await invoke<ReceiveSettings>("receive_settings");
    } catch (error) {
      report(`无法读取接收目录：${String(error)}`, true);
    }
  }
  async function loadReceipts(messages: string[]) {
    if (!native || !messages.length) return;
    // Timeline pagination may keep more than one page in memory.
    for (let offset = 0; offset < messages.length; offset += 200) {
      const batch = messages.slice(offset, offset + 200);
      const rows = await invoke<SavedFile[]>("received_files", {
        messages: batch,
      });
      for (const message of batch) {
        for (const [id, row] of records)
          if (row.message_id === message) records.delete(id);
      }
      for (const row of rows || [])
        records.set(key(row.message_id, row.file_id), row);
    }
  }
  async function changeDirectory(reset = false) {
    if (busy.value) return;
    busy.value = true;
    try {
      settings.value = await invoke<ReceiveSettings>(
        reset ? "reset_receive_directory" : "choose_receive_directory",
      );
    } catch (error) {
      report(String(error), true);
    } finally {
      busy.value = false;
    }
  }
  async function openDirectory() {
    try {
      await invoke("open_receive_directory");
    } catch (error) {
      report(String(error), true);
    }
  }
  async function openFile(message: string, file: string, folder = false) {
    try {
      await invoke("open_received_file", {
        messageId: message,
        fileId: file,
        folder,
      });
    } catch (error) {
      report(String(error), true);
    }
  }
  async function saveFile(message: string, file: MessageFile) {
    try {
      await invoke("save_received_file", {
        transferId: file.transfer_id,
        fileId: file.id,
      });
      await loadReceipts([message]);
      report("已加入保存队列");
    } catch (error) {
      report(String(error), true);
    }
  }
  return {
    settings,
    busy,
    receipt,
    loadSettings,
    loadReceipts,
    changeDirectory,
    openDirectory,
    openFile,
    saveFile,
  };
}
