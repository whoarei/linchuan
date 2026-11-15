import { test, expect, firefox } from "@playwright/test";
import { execFile } from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { promisify } from "node:util";

const execute = promisify(execFile);
async function boot(index = 0) {
  return JSON.parse(await readFile(`.playwright/bootstrap-${index}.json`, "utf8"));
}

test("untrusted browser rejects the device certificate", async ({ page }) => {
  const bootstrap = await boot();
  await expect(page.goto(bootstrap.https_address)).rejects.toThrow(/CERT_AUTHORITY_INVALID/);
});

test("an isolated trusted CA enables HTTPS pairing, secure context, WSS and intact file downloads", async ({ request }) => {
  test.setTimeout(90_000);
  const bootstrap = await boot();
  const other = await boot(1);
  const profile = await mkdtemp(path.join(tmpdir(), "linchuan-https-browser-"));
  const certificate = await request.get(bootstrap.address + "/api/certificate");
  expect(certificate.ok()).toBeTruthy();
  const pem = await certificate.text();
  expect(pem).toContain("BEGIN CERTIFICATE");
  expect(pem).not.toContain("PRIVATE KEY");
  await writeFile(path.join(profile, "linchuan-ca.crt"), pem);
  // Trust is confined to this disposable Firefox profile, never the host OS or
  // the user's browser. No certificate-error bypass flags are used.
  await execute("certutil", ["-N", "-d", `sql:${profile}`, "--empty-password"]);
  await execute("certutil", ["-A", "-d", `sql:${profile}`, "-n", "Linchuan test device", "-t", "C,,", "-i", path.join(profile, "linchuan-ca.crt")]);
  let context = await firefox.launchPersistentContext(profile, { headless: true, ignoreHTTPSErrors: false });
  try {
    const page = await context.newPage();
    const response = await page.goto(bootstrap.https_address);
    expect(response?.ok()).toBeTruthy();
    expect(await page.evaluate(() => ({ secure: window.isSecureContext, media: typeof navigator.mediaDevices?.getUserMedia, protocol: location.protocol })))
      .toEqual({ secure: true, media: "function", protocol: "https:" });
    await page.getByText("HTTPS 连接说明", { exact: true }).click();
    await expect(page.getByTestId("secure-context")).toContainText("已就绪");
    await page.getByLabel("你的设备名称").fill("HTTPS 浏览器");
    await page.getByLabel("六位配对码").fill(bootstrap.pairing_code);
    const connected = page.waitForEvent("websocket", { predicate: socket => socket.url() === bootstrap.https_address.replace("https://", "wss://") + "/api/events" });
    await page.getByRole("button", { name: "连接设备", exact: true }).click();
    const socket = await connected;
    let notifications = 0;
    socket.on("framereceived", frame => { if (frame.payload.toString() === "changed") notifications++; });
    await expect(page.getByRole("navigation", { name: "主要功能" })).toBeVisible();
    const browserIdentity = await page.evaluate(() => localStorage.getItem("linchuan-token"));
    await page.getByRole("button", { name: "连接 测试电脑", exact: true }).click();
    await page.getByRole("textbox", { name: "发送文字" }).fill("经过 HTTPS 的文字和文件");
    const bytes = Buffer.from("HTTPS 文件内容\n保持原始字节\u0000");
    await page.locator("input[type=file]").setInputFiles({ name: "HTTPS 测试.txt", mimeType: "text/plain", buffer: bytes });
    await page.getByRole("button", { name: "发送", exact: true }).click();
    const headers = { Authorization: `Bearer ${bootstrap.token}` };
    let transfer: any;
    await expect.poll(async () => {
      const response = await request.get(bootstrap.address + "/api/state", { headers });
      transfer = (await response.json()).transfers.find((item: any) => item.text === "经过 HTTPS 的文字和文件");
      return Boolean(transfer);
    }).toBeTruthy();
    const decision = await request.post(`${bootstrap.address}/api/transfers/${transfer.id}/decision`, { headers, data: { accept: true } });
    expect(decision.ok()).toBeTruthy();
    const message = page.locator(".message").filter({ hasText: "经过 HTTPS 的文字和文件" });
    await expect(message.getByText("已送达", { exact: true })).toBeVisible();
    await expect.poll(() => notifications).toBeGreaterThan(0);
    const downloading = page.waitForEvent("download");
    await message.getByRole("button", { name: "下载 HTTPS 测试.txt" }).click();
    const download = await downloading;
    expect(await readFile((await download.path())!)).toEqual(bytes);
    await page.getByRole("button", { name: "设置", exact: true }).click();
    await page.getByText("证书与首次信任", { exact: true }).click();
    const exporting = page.waitForEvent("download");
    await page.getByRole("button", { name: "下载本设备证书", exact: true }).click();
    const exported = await exporting;
    expect(await readFile((await exported.path())!, "utf8")).toBe(pem);
    const foreign = await context.newPage();
    await expect(foreign.goto(other.https_address)).rejects.toThrow(/SEC_ERROR_UNKNOWN_ISSUER|SEC_ERROR_UNTRUSTED_ISSUER|certificate/i);
    await context.close();
    context = await firefox.launchPersistentContext(profile, { headless: true, ignoreHTTPSErrors: false });
    const reopened = await context.newPage();
    await reopened.goto(bootstrap.https_address);
    await expect(reopened.getByRole("navigation", { name: "主要功能" })).toBeVisible();
    expect(await reopened.evaluate(() => localStorage.getItem("linchuan-token"))).toBe(browserIdentity);
  } finally {
    await context.close();
    await rm(profile, { recursive: true, force: true });
  }
});
