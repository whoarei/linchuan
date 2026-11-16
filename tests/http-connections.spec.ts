import { test, expect } from "@playwright/test";
import { readFile } from "node:fs/promises";

test.use({ launchOptions: { args: ["--no-proxy-server"] } });

async function boot() {
  return JSON.parse(await readFile(".playwright/bootstrap-3.json", "utf8"));
}

test("LAN HTTP browser pairs without certificates and transfers through WS", async ({ page, request }) => {
  const bootstrap = await boot();
  const info = await (await request.get(bootstrap.address + "/api/info")).json();
  expect(info.protocol).toBe(1);
  expect(info.certificate_fingerprint).toBeUndefined();
  // The managed local browser forwards loopback, but its proxy cannot reach the
  // workspace's LAN interface. CI exercises the advertised LAN address; local
  // runs exercise the same HTTP routes through the forwarded loopback address.
  const address = process.env.CI ? info.addresses[0] : bootstrap.address;
  expect(address).toMatch(/^http:\/\//);
  expect((await page.goto(address))?.ok()).toBeTruthy();
  await expect(page.getByText("同一局域网 · 无需账号 · HTTP 传输", { exact: true })).toBeVisible();
  await expect(page.getByText("HTTPS 连接说明", { exact: true })).toHaveCount(0);
  await page.getByLabel("你的设备名称").fill("HTTP 浏览器");
  await page.getByLabel("六位配对码").fill(bootstrap.pairing_code);
  const connecting = page.waitForEvent("websocket", { predicate: socket => socket.url() === address.replace("http://", "ws://") + "/api/events" });
  await page.getByRole("button", { name: "连接设备", exact: true }).click();
  const socket = await connecting;
  let notifications = 0;
  socket.on("framereceived", frame => { if (frame.payload.toString() === "changed") notifications++; });
  await expect(page.getByRole("navigation", { name: "主要功能" })).toBeVisible();
  const credential = await page.evaluate(() => localStorage.getItem("linchuan-token"));
  await expect(page.getByRole("heading",{name:info.name,exact:true})).toBeVisible();
  await page.getByRole("textbox", { name: "发送文字" }).fill("直接通过 HTTP 发送");
  const bytes = Buffer.from("HTTP 文件完整性\n\u0000");
  await page.locator("input[type=file]").setInputFiles({ name: "HTTP 测试.txt", mimeType: "text/plain", buffer: bytes });
  await page.getByRole("button", { name: "发送", exact: true }).click();
  const headers = { Authorization: `Bearer ${bootstrap.token}` };
  let transfer: any;
  await expect.poll(async () => {
    const state = await (await request.get(bootstrap.address + "/api/state", { headers })).json();
    transfer = state.transfers.find((item: any) => item.text === "直接通过 HTTP 发送");
    return Boolean(transfer);
  }).toBeTruthy();
  expect((await request.post(`${bootstrap.address}/api/transfers/${transfer.id}/decision`, { headers, data: { accept: true } })).ok()).toBeTruthy();
  const message = page.locator(".message").filter({ hasText: "直接通过 HTTP 发送" });
  await expect(message.getByText("已送达", { exact: true })).toBeVisible();
  await expect.poll(() => notifications).toBeGreaterThan(0);
  const downloading = page.waitForEvent("download");
  await message.getByRole("button", { name: "下载 HTTP 测试.txt" }).click();
  expect(await readFile((await (await downloading).path())!)).toEqual(bytes);
  await page.reload();
  await expect(page.getByRole("navigation", { name: "主要功能" })).toBeVisible();
  expect(await page.evaluate(() => localStorage.getItem("linchuan-token"))).toBe(credential);
  const context = await page.evaluate(() => ({ secure: window.isSecureContext, loopback: ["localhost", "127.0.0.1"].includes(location.hostname) }));
  if (!context.loopback) expect(context.secure).toBe(false);
  await page.getByRole("button", { name: "设置", exact: true }).click();
  await expect(page.getByRole("region", { name: "连接安全" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "下载本设备证书", exact: true })).toHaveCount(0);
});

test("owner settings and manual connection present HTTP without certificate steps", async ({ page }) => {
  const bootstrap = await boot();
  await page.addInitScript(token => sessionStorage.setItem("linchuan-token", token), bootstrap.token);
  await page.goto(bootstrap.address);
  await page.getByRole("button", { name: "设置", exact: true }).click();
  await expect(page.getByRole("button", { name: "下载本设备证书", exact: true })).toHaveCount(0);
  await page.getByRole("button", { name: "设备", exact: true }).click();
  await page.getByRole("button", { name: "通过地址连接", exact: true }).first().click();
  await expect(page.getByLabel("对方设备地址")).toHaveAttribute("placeholder", "http://192.168.1.20:52783");
});
