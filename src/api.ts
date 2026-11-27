import { invoke, isTauri } from "@tauri-apps/api/core";
import { RequestTimeoutError, withDeadline } from "./request";
import type { ReceiveConflict } from "./types";
export const native = isTauri();
let base = native ? "" : window.location.origin;
const credentialKey = "linchuan-token";
function storedCredential() {
  let value = "";
  try {
    value = localStorage.getItem(credentialKey) || "";
  } catch {}
  if (!value) {
    try {
      value = sessionStorage.getItem(credentialKey) || "";
    } catch {}
    if (value) {
      try {
        localStorage.setItem(credentialKey, value);
      } catch {}
    }
  }
  if (value) {
    try {
      sessionStorage.setItem(credentialKey, value);
    } catch {}
  }
  return value;
}
let credential = native ? "" : storedCredential();
if (!native)
  window.addEventListener("storage", (event) => {
    if (
      event.storageArea !== localStorage ||
      (event.key !== credentialKey && event.key !== null)
    )
      return;
    const next = event.newValue || "";
    if (next === credential) return;
    credential = next;
    try {
      if (next) sessionStorage.setItem(credentialKey, next);
      else sessionStorage.removeItem(credentialKey);
    } catch {}
    window.dispatchEvent(new Event("linchuan-session-change"));
  });
export let pairingCode = "";
export let browserHttps: { address: string; pairing_code: string } | undefined;
export async function bootstrap() {
  if (native) {
    const result = await withDeadline(() =>
      invoke<{
        address: string;
        token: string;
        pairing_code: string;
        browser_https?: typeof browserHttps | null;
      }>("bootstrap"),
    );
    base = result.address;
    credential = result.token;
    pairingCode = result.pairing_code;
    browserHttps = result.browser_https ?? undefined;
  }
}
export const address = () => base;
export const token = () => credential;
export function setToken(value: string) {
  credential = value;
  if (!native) {
    try {
      localStorage.setItem(credentialKey, value);
    } catch {}
    try {
      sessionStorage.setItem(credentialKey, value);
    } catch {}
  }
}
export function forget(expected = credential) {
  if (expected !== credential) return;
  credential = "";
  if (!native) {
    try {
      if (localStorage.getItem(credentialKey) === expected)
        localStorage.removeItem(credentialKey);
    } catch {}
    try {
      sessionStorage.removeItem(credentialKey);
    } catch {}
  }
}
export class ApiError extends Error {
  constructor(
    message: string,
    public status: number,
    public conflicts?: ReceiveConflict[],
  ) {
    super(message);
  }
}
export async function api<T = any>(
  path: string,
  body?: unknown,
  options?: { keepalive?: boolean; anonymous?: boolean },
): Promise<T> {
  try {
    return await withDeadline(async (signal) => {
      const response = await fetch(base + path, {
        signal,
        keepalive: options?.keepalive,
        method: body === undefined ? "GET" : "POST",
        headers: {
          ...(credential && !options?.anonymous ? { Authorization: `Bearer ${credential}` } : {}),
          ...(body === undefined ? {} : { "Content-Type": "application/json" }),
        },
        body: body === undefined ? undefined : JSON.stringify(body),
      });
      if (!response.ok) {
        const value = await response.json().catch(() => ({}));
        throw new ApiError(value.error || "请求失败", response.status, value.conflicts);
      }
      return await response.json();
    });
  } catch (error) {
    if (error instanceof RequestTimeoutError && body !== undefined) {
      // The server may have processed the write before the response timed out.
      throw new Error("请求超时，操作结果尚未确认。请先查看当前状态，勿重复提交。");
    }
    throw error;
  }
}
export function upload(
  path: string,
  file: File,
  onProgress: (bytes: number) => void,
  signal: AbortSignal,
  options?: { anonymous?: boolean },
): Promise<void> {
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open("PUT", base + path);
    if (!options?.anonymous) xhr.setRequestHeader("Authorization", `Bearer ${credential}`);
    xhr.timeout = 0;
    let lastLoaded = 0;
    let idleTimer: ReturnType<typeof setTimeout>;
    let idle = false;
    const resetIdle = () => {
      clearTimeout(idleTimer);
      idleTimer = setTimeout(() => {
        idle = true;
        xhr.abort();
      }, 5 * 60_000);
    };
    const abort = () => xhr.abort();
    signal.addEventListener("abort", abort, { once: true });
    const done = () => {
      clearTimeout(idleTimer);
      signal.removeEventListener("abort", abort);
    };
    xhr.upload.onprogress = (e) => {
      if (e.loaded > lastLoaded) {
        lastLoaded = e.loaded;
        resetIdle();
      }
      onProgress(e.loaded);
    };
    xhr.upload.onload = resetIdle;
    xhr.onload = () => {
      done();
      if (xhr.status >= 200 && xhr.status < 300) resolve();
      else {
        let message = "文件上传失败";
        try {
          message = JSON.parse(xhr.responseText).error || message;
        } catch {}
        reject(new Error(message));
      }
    };
    xhr.onerror = () => {
      done();
      reject(new Error("上传连接中断，请重新发送"));
    };
    xhr.ontimeout = () => {
      done();
      reject(new Error("上传超时"));
    };
    xhr.onabort = () => {
      done();
      reject(
        new Error(idle ? "连续 5 分钟无上传进度，请重新发送" : "已取消上传"),
      );
    };
    if (signal.aborted) {
      done();
      reject(new Error("已取消上传"));
      return;
    }
    resetIdle();
    xhr.send(file);
  });
}
