import { test, expect, type Browser, type Page } from "@playwright/test";
import { readFile } from "node:fs/promises";
async function boot(index = 0) {
  return JSON.parse(
    await readFile(`.playwright/bootstrap-${index}.json`, "utf8"),
  );
}
async function owner(browser: Browser, index = 0) {
  const b = await boot(index);
  const context = await browser.newContext();
  await context.addInitScript(
    (token) => sessionStorage.setItem("linchuan-token", token),
    b.token,
  );
  const page = await context.newPage();
  await page.goto(b.address);
  await expect(
    page.getByRole("navigation", { name: "主要功能" }),
  ).toBeVisible();
  return { page, context, b };
}
async function pair(page: Page, name: string) {
  const b = await boot();
  await page.goto(b.address);
  await page.getByLabel("你的设备名称").fill(name);
  await page.getByLabel("六位配对码").fill(b.pairing_code);
  await page.getByRole("button", { name: "连接设备", exact: true }).click();
  await expect(
    page.getByRole("navigation", { name: "主要功能" }),
  ).toBeVisible();
}
async function addPeer(page: Page, address: string) {
  await page.getByRole("button", { name: "添加", exact: true }).click();
  await page
    .getByRole("button", { name: "通过地址连接", exact: true })
    .first()
    .click();
  await page.getByLabel("对方设备地址").fill(address);
  await page.getByRole("button", { name: "连接设备", exact: true }).click();
  await expect(page.getByRole("textbox", { name: "发送文字" })).toBeVisible();
}

test("browser text and Unicode file are received in a per-device conversation and downloaded byte-for-byte", async ({
  browser,
  page,
}) => {
  const host = await owner(browser);
  await pair(page, "网页手机");
  await page
    .getByRole("button", { name: "连接 测试电脑", exact: true })
    .click();
  await page
    .getByRole("textbox", { name: "发送文字" })
    .fill("来自浏览器的一段文字");
  const bytes = Buffer.from("邻传原始内容\n图片、文字与文件");
  await page
    .locator("input[type=file]")
    .setInputFiles({
      name: "你好 邻传.txt",
      mimeType: "text/plain",
      buffer: bytes,
    });
  await page.getByRole("button", { name: "发送", exact: true }).click();
  await host.page
    .getByRole("button", { name: "打开 网页手机", exact: true })
    .click();
  const card = host.page
    .locator(".message")
    .filter({ hasText: "来自浏览器的一段文字" });
  await card.getByRole("button", { name: "接收", exact: true }).click();
  const senderCard = page
    .locator(".message")
    .filter({ hasText: "来自浏览器的一段文字" });
  await expect(senderCard.getByText("已送达", { exact: true })).toBeVisible();
  const downloading = page.waitForEvent("download");
  await senderCard.getByRole("button", { name: "下载 你好 邻传.txt" }).click();
  const download = await downloading;
  expect(download.suggestedFilename()).toBe("你好 邻传.txt");
  expect(await readFile((await download.path())!)).toEqual(bytes);
  await host.context.close();
});
test("two applications exchange directly in the new three-column layout", async ({
  browser,
}) => {
  const a = await owner(browser);
  const b = await owner(browser, 1);
  await addPeer(a.page, b.b.address);
  await a.page
    .getByRole("textbox", { name: "发送文字" })
    .fill("应用之间直接传输");
  await a.page.getByRole("button", { name: "发送", exact: true }).click();
  await b.page
    .getByRole("button", { name: "打开 测试电脑", exact: true })
    .click();
  const card = b.page
    .locator(".message")
    .filter({ hasText: "应用之间直接传输" });
  await card.getByRole("button", { name: "接收", exact: true }).click();
  await expect(
    a.page
      .locator(".message")
      .filter({ hasText: "应用之间直接传输" })
      .getByText("已送达", { exact: true }),
  ).toBeVisible();
  await expect(a.page.locator(".function-rail")).toBeVisible();
  await expect(a.page.locator(".conversation-panel")).toBeVisible();
  await expect(a.page.locator(".chat-panel")).toBeVisible();
  await a.context.close();
  await b.context.close();
});
test("group invitation, shared timeline and per-recipient acceptance work for browser members", async ({
  browser,
}) => {
  const host = await owner(browser);
  const ca = await browser.newContext();
  const cb = await browser.newContext();
  const a = await ca.newPage();
  const b = await cb.newPage();
  await pair(a, "群成员甲");
  await pair(b, "群成员乙");
  await host.page.getByRole("button", { name: "添加", exact: true }).click();
  await host.page
    .getByRole("button", { name: "新建群聊", exact: true })
    .click();
  await host.page.getByLabel("群名称").fill("资料共享群");
  await host.page.getByRole("checkbox", { name: /群成员甲/ }).check();
  await host.page.getByRole("checkbox", { name: /群成员乙/ }).check();
  await host.page
    .getByRole("button", { name: "创建群聊", exact: true })
    .click();
  for (const p of [a, b]) {
    await p
      .getByRole("button", { name: "打开 资料共享群", exact: true })
      .click();
    await p.getByRole("button", { name: "加入群聊", exact: true }).click();
  }
  await host.page
    .getByRole("textbox", { name: "发送文字" })
    .fill("群里只有一条消息");
  await host.page
    .locator("input[type=file]")
    .setInputFiles({
      name: "群资料.txt",
      mimeType: "text/plain",
      buffer: Buffer.from("group bytes"),
    });
  await host.page.getByRole("button", { name: "发送", exact: true }).click();
  for (const p of [a, b]) {
    const message = p
      .locator(".message")
      .filter({ hasText: "群里只有一条消息" });
    await message.getByRole("button", { name: "接收", exact: true }).click();
  }
  const bubble = host.page
    .locator(".message")
    .filter({ hasText: "群里只有一条消息" });
  await expect(bubble).toHaveCount(1);
  await expect(bubble.getByText("已送达", { exact: true })).toBeVisible();
  await bubble.getByRole("button", { name: "已送达", exact: true }).click();
  await expect(bubble.getByText("已完成", { exact: true })).toHaveCount(2);
  await host.page.screenshot({
    path: ".playwright/desktop.png",
    fullPage: true,
  });
  await host.context.close();
  await ca.close();
  await cb.close();
});
test("settings exposes device rename; drafts and history survive a page reload", async ({
  browser,
}) => {
  const host = await owner(browser);
  await host.page
    .getByRole("button", { name: "打开 资料共享群", exact: true })
    .click();
  await host.page
    .getByRole("textbox", { name: "发送文字" })
    .fill("还没有发送的草稿");
  await host.page.getByRole("button", { name: "设置", exact: true }).click();
  await host.page
    .getByLabel("设备名称", { exact: true })
    .fill("重命名后的电脑");
  await host.page
    .getByRole("button", { name: "保存名称", exact: true })
    .click();
  await expect(host.page.getByRole("status")).toHaveText("设备名称已保存");
  await host.page.reload();
  await host.page
    .getByRole("button", { name: "打开 资料共享群", exact: true })
    .click();
  await expect(
    host.page.getByRole("textbox", { name: "发送文字" }),
  ).toHaveValue("还没有发送的草稿");
  await expect(
    host.page.locator(".message").filter({ hasText: "群里只有一条消息" }),
  ).toHaveCount(1);
  await host.page.getByRole("button", { name: "关于", exact: true }).click();
  await expect(
    host.page.getByRole("heading", { name: "邻传 · Linchuan" }),
  ).toBeVisible();
  await expect(host.page.getByText("0.2.0", { exact: true })).toBeVisible();
  await host.context.close();
});
test("mobile has list-to-conversation navigation and no horizontal overflow", async ({
  browser,
}) => {
  const context = await browser.newContext({
    viewport: { width: 390, height: 844 },
    isMobile: true,
    hasTouch: true,
  });
  const page = await context.newPage();
  await pair(page, "移动网页");
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page
    .getByRole("button", { name: "连接 重命名后的电脑", exact: true })
    .click();
  await expect(page.getByRole("textbox", { name: "发送文字" })).toBeVisible();
  await expect(page.getByRole("navigation", { name: "主要功能" })).toBeHidden();
  await page.getByRole("textbox", { name: "发送文字" }).fill("手机会话");
  await page.getByRole("button", { name: "发送", exact: true }).click();
  await expect(
    page.locator(".message").filter({ hasText: "手机会话" }),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({ path: ".playwright/mobile.png", fullPage: true });
  await page.getByRole("button", { name: "返回设备列表" }).click();
  await expect(
    page.getByRole("navigation", { name: "主要功能" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "设置", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "设置", exact: true }),
  ).toBeVisible();
  await context.close();
});
test("wrong pairing code shows a clear error", async ({ page }) => {
  const b = await boot();
  await page.goto(b.address);
  await page.getByLabel("你的设备名称").fill("错误配对");
  await page
    .getByLabel("六位配对码")
    .fill(b.pairing_code === "000000" ? "111111" : "000000");
  await page.getByRole("button", { name: "连接设备", exact: true }).click();
  await expect(page.getByRole("alert")).toBeVisible();
});
