import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { native } from "./api";

interface DesktopSettings {
  supported: boolean;
  autostart: boolean;
  close_to_tray: boolean;
  tray_available: boolean;
}

export function useDesktopSettings(
  report: (message: string, error?: boolean) => void,
) {
  const settings = ref<DesktopSettings>();
  const busy = ref(false);
  let revision = 0;
  async function load() {
    if (!native || busy.value) return;
    const request = ++revision;
    try {
      const value = await invoke<DesktopSettings>("desktop_settings");
      if (request === revision) settings.value = value;
    } catch (error) {
      report(`无法读取后台设置：${String(error)}`, true);
    }
  }
  async function set(
    command: string,
    field: "autostart" | "close_to_tray",
    enabled: boolean,
  ) {
    if (busy.value) return;
    ++revision;
    const previous = settings.value;
    if (previous) settings.value = { ...previous, [field]: enabled };
    busy.value = true;
    try {
      settings.value = await invoke<DesktopSettings>(command, { enabled });
    } catch (error) {
      settings.value = previous;
      report(String(error), true);
      try {
        settings.value = await invoke<DesktopSettings>("desktop_settings");
      } catch {}
    } finally {
      busy.value = false;
    }
  }
  async function quit() {
    try {
      await invoke("desktop_quit");
    } catch (error) {
      report(String(error), true);
    }
  }
  return {
    settings,
    busy,
    load,
    quit,
    setAutostart: (enabled: boolean) =>
      set("desktop_set_autostart", "autostart", enabled),
    setCloseToTray: (enabled: boolean) =>
      set("desktop_set_close_to_tray", "close_to_tray", enabled),
  };
}
