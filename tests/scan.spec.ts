import { expect, test, type Browser } from "@playwright/test";
import { readFile } from "node:fs/promises";
import path from "node:path";

async function publisher(browser: Browser, viewport = { width: 1100, height: 820 }) {
  const boot = JSON.parse(await readFile(".playwright/bootstrap-2.json", "utf8"));
  const context = await browser.newContext({ viewport });
  await context.addInitScript((boot) => {
    Object.assign(window, {
      isTauri: true,
      __TAURI_INTERNALS__: {
        invoke: async (command: string) => {
          if (command === "bootstrap") return boot;
          if (command === "receive_settings") return { directory: "", label: "" };
          return false;
        },
      },
    });
  }, boot);
  const page = await context.newPage();
  await page.goto(boot.address);
  return { page, context, boot };
}

test("the publisher creates a pickup QR page for unpaired mobile visitors and can close it", async ({ browser }) => {
  const { page, context, boot } = await publisher(browser);
  const phone = await browser.newContext({ viewport: { width: 360, height: 720 }, isMobile: true });
  const guest = await phone.newPage();
  const nav = page.getByRole("navigation", { name: "主要功能" });
  const deviceNav = nav.getByRole("button", { name: "设备", exact: true });
  const scanNav = nav.getByRole("button", { name: "收发", exact: true });
  await expect(page.locator(".conversation-panel")).toBeVisible();
  const device = (await deviceNav.boundingBox())!;
  const scan = (await scanNav.boundingBox())!;
  expect(scan.x).toBe(device.x);
  expect(scan.y).toBeGreaterThanOrEqual(device.y + device.height);
  await expect(nav.locator(".rail-button")).toHaveText(["设备", "收发", "设置", "关于"]);
  await scanNav.click();
  await expect(scanNav).toHaveAttribute("aria-current", "page");
  const work = page.getByRole("main", { name: "收发" });
  await expect(page.locator(".conversation-panel")).toHaveCount(0);
  await expect(work.getByRole("button", { name: "收文件", exact: true })).toBeVisible();
  await work.getByRole("button", { name: "发资料", exact: true }).click();
  await expect(work.getByRole("heading", { name: "新建取件页" })).toBeVisible();
  await work.getByLabel("页面标题").fill("会议资料");
  await work.getByLabel("给访客的说明（选填）").fill("请查看会议纪要");
  await work.getByLabel("链接（每行一个，选填）").fill("https://example.org/minutes");
  await work.getByLabel("有效期").selectOption({ label: "15 分钟" });
  await work.getByRole("button", { name: "生成取件码" }).click();
  await expect(work.getByRole("img", { name: "扫码取件二维码" })).toHaveAttribute("src", /^data:image\/png/);
  const shared = new URL(await work.getByLabel("访问链接").inputValue());
  expect(shared.pathname).toMatch(/^\/scan\/[a-f0-9]{64}$/);
  await guest.goto(boot.address + shared.pathname);
  await expect(guest.getByRole("heading", { name: "会议资料" })).toBeVisible();
  await expect(guest.getByRole("link", { name: "https://example.org/minutes" })).toHaveAttribute("href", "https://example.org/minutes");
  await expect(guest.getByLabel("六位配对码")).toHaveCount(0);
  expect(await guest.evaluate(() => document.documentElement.scrollWidth)).toBe(360);
  await work.getByRole("button", { name: "关闭扫码页面" }).click();
  await expect(work.getByText("页面已结束")).toBeVisible();
  await expect(guest.getByRole("alert")).toHaveText("此扫码页面已关闭或已过期。");
  await work.getByRole("button", { name: "返回收发" }).click();
  await expect(work.getByRole("button", { name: "发资料", exact: true })).toBeVisible();
  await deviceNav.click();
  await expect(page.locator(".conversation-panel")).toBeVisible();
  await phone.close();
  await context.close();
});

test("different unpaired visitors submit privately and same-named files are saved separately", async ({ browser, request }) => {
  const { page, context, boot } = await publisher(browser, { width: 360, height: 720 });
  await page.getByRole("button", { name: "收发", exact: true }).click();
  const work = page.getByRole("main", { name: "收发" });
  await work.getByRole("button", { name: "收文件", exact: true }).click();
  await expect(work.getByRole("heading", { name: "新建交件页" })).toBeVisible();
  await work.getByLabel("页面标题").fill("收会议照片");
  await work.getByRole("button", { name: "生成交件码" }).click();
  const shared = new URL(await work.getByLabel("访问链接").inputValue());
  await expect(work.getByRole("img", { name: "扫码交件二维码" })).toBeVisible();
  expect(await work.evaluate((el) => el.scrollWidth - el.clientWidth)).toBe(0);
  const guests = await Promise.all([browser.newContext(), browser.newContext()]);
  await Promise.all(guests.map(async (guest, index) => {
    const visitor = await guest.newPage();
    await visitor.goto(boot.address + shared.pathname);
    await visitor.getByLabel("提交人（选填）").fill(`参与者 ${index}`);
    await visitor.getByLabel("选择要提交的文件或照片").setInputFiles({
      name: "照片.jpg",
      mimeType: "image/jpeg",
      buffer: Buffer.from(`photo-${index}`),
    });
    await visitor.getByRole("button", { name: "提交文件" }).click();
    await expect(visitor.getByRole("status")).toHaveText("本次文件已全部提交。");
    await expect(visitor.getByText(`参与者 ${1 - index}`, { exact: false })).toHaveCount(0);
    await expect(visitor.getByText("已收文件")).toHaveCount(0);
  }));
  await expect(work.getByRole("heading", { name: "已收文件 2" })).toBeVisible();
  await expect(work.getByText("参与者 0", { exact: false })).toBeVisible();
  await expect(work.getByText("参与者 1", { exact: false })).toBeVisible();
  await expect(work.getByText("照片 (1).jpg", { exact: true })).toBeVisible();
  await work.getByRole("button", { name: "返回收发" }).click();
  await expect(work.getByText("页面开放中")).toBeVisible();
  await work.getByRole("button", { name: "收文件", exact: true }).click();
  await expect(work.getByRole("heading", { name: "已收文件 2" })).toBeVisible();
  const saved = await Promise.all(["照片.jpg", "照片 (1).jpg"].map((name) =>
    readFile(path.join(".playwright", "device-2", "received", name), "utf8")));
  expect(saved.sort()).toEqual(["photo-0", "photo-1"]);
  const view = await (await request.get(boot.address + "/api/scan/visit/" + shared.pathname.split("/").pop())).json();
  expect(view.submissions).toBeUndefined();
  expect(view.directory).toBeUndefined();
  expect((await request.get(boot.address + "/api/scan")).status()).toBe(401);
  await Promise.all(guests.map((guest) => guest.close()));
  await context.close();
});
