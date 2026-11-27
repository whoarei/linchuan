import type { FileMeta } from "./types";

export type ScanKind = "pickup" | "dropoff";

export interface ScanView {
  kind: ScanKind;
  title: string;
  note: string;
  expires_at: number;
  files: FileMeta[];
  links: string[];
}

export interface ScanReceipt {
  name: string;
  sender: string;
  size: number;
  submitted_at: number;
}

export interface ScanPage extends ScanView {
  id: string;
  active: boolean;
  directory: string | null;
  submissions: ScanReceipt[];
}

export function scanSize(bytes: number) {
  const units = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
  let unit = 0;
  while (bytes >= 1024 && unit < units.length - 1) {
    bytes /= 1024;
    unit++;
  }
  return `${unit ? bytes.toFixed(1) : bytes} ${units[unit]}`;
}

export function scanId() {
  return Array.from(crypto.getRandomValues(new Uint8Array(16)), (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
}
