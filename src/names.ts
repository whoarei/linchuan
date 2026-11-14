const automaticKey = "linchuan-browser-auto-name";
const preferredKey = "linchuan-browser-name";

function storedName(key: string): string {
  try {
    const name = localStorage.getItem(key)?.trim() || "";
    return name &&
      [...name].length <= 64 &&
      !/[\u0000-\u001f\u007f-\u009f]/.test(name)
      ? name
      : "";
  } catch {
    return "";
  }
}

function storeName(key: string, name: string) {
  try {
    localStorage.setItem(key, name);
  } catch {
    // Pairing still works when browser storage is unavailable.
  }
}

export function browserIdentity() {
  let automatic = storedName(automaticKey);
  if (!automatic) {
    const ua = navigator.userAgent;
    const os = /Android/.test(ua)
      ? "Android"
      : /iPhone|iPad|iPod/.test(ua) ||
          (/Macintosh/.test(ua) && navigator.maxTouchPoints > 1)
        ? "iOS"
        : /Windows/.test(ua)
          ? "Windows"
          : /CrOS/.test(ua)
            ? "ChromeOS"
            : /Macintosh|Mac OS X/.test(ua)
              ? "Mac"
              : /Linux/.test(ua)
                ? "Linux"
                : "";
    const browser = /Edg\/|EdgA\/|EdgiOS\//.test(ua)
      ? "Edge"
      : /OPR\/|Opera/.test(ua)
        ? "Opera"
        : /SamsungBrowser\//.test(ua)
          ? "三星浏览器"
          : /Firefox\/|FxiOS\//.test(ua)
            ? "Firefox"
            : /Chrome\/|CriOS\//.test(ua)
              ? "Chrome"
              : /Safari\//.test(ua)
                ? "Safari"
                : "浏览器";
    const suffix = crypto
      .getRandomValues(new Uint16Array(1))[0]
      .toString(16)
      .padStart(4, "0")
      .toUpperCase();
    automatic = [os, browser, suffix].filter(Boolean).join(" · ");
    storeName(automaticKey, automatic);
  }
  return { automatic, preferred: storedName(preferredKey) || automatic };
}

export function rememberBrowserName(name: string) {
  storeName(preferredKey, name);
}
