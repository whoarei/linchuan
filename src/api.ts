import { invoke, isTauri } from "@tauri-apps/api/core";
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
export async function bootstrap() {
  if (native) {
    const result = await invoke<{
      address: string;
      token: string;
      pairing_code: string;
    }>("bootstrap");
    base = result.address;
    credential = result.token;
    pairingCode = result.pairing_code;
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
  ) {
    super(message);
  }
}
export async function api<T = any>(path: string, body?: unknown): Promise<T> {
  const response = await fetch(base + path, {
    method: body === undefined ? "GET" : "POST",
    headers: {
      ...(credential ? { Authorization: `Bearer ${credential}` } : {}),
      ...(body === undefined ? {} : { "Content-Type": "application/json" }),
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!response.ok) {
    const value = await response
      .json()
      .catch(() => ({ error: "请求失败，请检查连接" }));
    throw new ApiError(value.error || "请求失败", response.status);
  }
  return response.json();
}
export function upload(
  path: string,
  file: File,
  onProgress: (bytes: number) => void,
  signal: AbortSignal,
): Promise<void> {
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open("PUT", base + path);
    xhr.setRequestHeader("Authorization", `Bearer ${credential}`);
    xhr.timeout = 3600_000;
    const abort = () => xhr.abort();
    signal.addEventListener("abort", abort, { once: true });
    const done = () => signal.removeEventListener("abort", abort);
    xhr.upload.onprogress = (e) => onProgress(e.loaded);
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
      reject(new Error("已取消上传"));
    };
    if (signal.aborted) {
      done();
      reject(new Error("已取消上传"));
      return;
    }
    xhr.send(file);
  });
}
