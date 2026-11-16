import { test, expect, type Browser, type Page } from "@playwright/test";
import { readFile } from "node:fs/promises";

async function boot() {
  return JSON.parse(await readFile(".playwright/bootstrap-2.json", "utf8"));
}
async function identity(page: Page) {
  return page.evaluate(
    async () =>
      (
        await (
          await fetch("/api/state", {
            headers: {
              Authorization: `Bearer ${sessionStorage.getItem("linchuan-token")}`,
            },
          })
        ).json()
      ).me,
  );
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
async function owner(browser: Browser) {
  const b = await boot();
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
  return { context, page, b };
}

test("browser reopens and new tabs reuse a temporary identity; logout revokes all tabs", async ({
  browser,
}) => {
  const context = await browser.newContext();
  let page = await context.newPage();
  await pair(page, "复用临时会话");
  const first = await identity(page);
  const credential = await page.evaluate(() =>
    localStorage.getItem("linchuan-token"),
  );
  expect(credential).toBeTruthy();
  await page.close();
  page = await context.newPage();
  await page.goto((await boot()).address);
  await expect(
    page.getByRole("navigation", { name: "主要功能" }),
  ).toBeVisible();
  expect((await identity(page)).id).toBe(first.id);
  const tab = await context.newPage();
  await tab.goto((await boot()).address);
  await expect(tab.getByRole("navigation", { name: "主要功能" })).toBeVisible();
  expect((await identity(tab)).id).toBe(first.id);
  await context.setOffline(true);
  await expect(page.getByTitle("连接中断", { exact: true })).toBeVisible();
  await context.setOffline(false);
  await expect(page.getByTitle("局域网已连接", { exact: true })).toBeVisible();
  expect((await identity(page)).id).toBe(first.id);
  await page.reload();
  await expect(
    page.getByRole("navigation", { name: "主要功能" }),
  ).toBeVisible();
  expect((await identity(page)).id).toBe(first.id);
  const b = await boot();
  const response = await page.request.get(b.address + "/api/state", {
    headers: { Authorization: `Bearer ${b.token}` },
  });
  expect(
    (await response.json()).devices.filter((d: any) => d.id === first.id),
  ).toHaveLength(1);
  await tab.getByRole("button", { name: "设置", exact: true }).click();
  await tab.getByRole("button", { name: "断开连接", exact: true }).click();
  await expect(tab.getByLabel("六位配对码")).toBeVisible();
  await expect(page.getByLabel("六位配对码")).toBeVisible();
  expect(
    await page.evaluate(() => [
      localStorage.getItem("linchuan-token"),
      sessionStorage.getItem("linchuan-token"),
    ]),
  ).toEqual([null, null]);
  expect(
    (
      await page.request.get(b.address + "/api/state", {
        headers: { Authorization: `Bearer ${credential}` },
      })
    ).status(),
  ).toBe(401);
  await context.close();
});

test("invalid saved credentials return to pairing and blocked browser storage still permits a temporary connection", async ({
  browser,
}) => {
  const b = await boot();
  const context = await browser.newContext();
  await context.addInitScript(() =>
    localStorage.setItem("linchuan-token", "expired-credential"),
  );
  const page = await context.newPage();
  await page.goto(b.address);
  await expect(page.getByLabel("六位配对码")).toBeVisible();
  expect(
    await page.evaluate(() => localStorage.getItem("linchuan-token")),
  ).toBeNull();
  await context.close();
  const blocked = await browser.newContext();
  await blocked.addInitScript(() => {
    for (const key of ["localStorage", "sessionStorage"])
      Object.defineProperty(window, key, {
        get() {
          throw new DOMException("Blocked", "SecurityError");
        },
      });
  });
  const temporary = await blocked.newPage();
  await pair(temporary, "无存储临时网页");
  await expect(
    temporary.getByRole("button", { name: "设置", exact: true }),
  ).toBeVisible();
  await blocked.close();
});

test("offline browser history folds away, empty sessions hide and drafts remain accessible", async ({
  browser,
}) => {
  const host = await owner(browser);
  const context = await browser.newContext();
  const page = await context.newPage();
  await pair(page, "历史临时网页");
  const original = await identity(page);
  await expect(page.getByRole("textbox", {name:"发送文字"})).toBeVisible();
  await page
    .getByRole("textbox", { name: "发送文字" })
    .fill("临时会话留下的文字");
  await page.getByRole("button", { name: "发送", exact: true }).click();
  await host.page
    .getByRole("button", { name: "打开 历史临时网页", exact: true })
    .click();
  await host.page.getByRole("button", { name: "接收", exact: true }).click();
  await page.getByRole("button", { name: "设置", exact: true }).click();
  await page.getByRole("button", { name: "断开连接", exact: true }).click();
  await expect(
    host.page.getByRole("button", { name: "打开 历史临时网页", exact: true }),
  ).toHaveCount(0);
  const history = host.page.getByRole("region", { name: "临时会话记录" });
  await expect(
    history.getByRole("button", { name: /临时会话记录/ }),
  ).toHaveAttribute("aria-expanded", "false");
  await history.getByRole("button", { name: /临时会话记录/ }).click();
  await history
    .getByRole("button", { name: "打开 历史临时网页", exact: true })
    .click();
  await expect(host.page.locator(".message-timeline")).toContainText(
    "临时会话留下的文字",
  );
  await history.getByRole("button", { name: /临时会话记录/ }).click();
  await pair(page, "历史临时网页");
  expect((await identity(page)).id).not.toBe(original.id);
  await expect(
    host.page.getByRole("button", { name: "连接 历史临时网页", exact: true }),
  ).toBeVisible();
  await host.page
    .getByRole("button", { name: "连接 历史临时网页", exact: true })
    .click();
  await page.getByRole("button", { name: "设置", exact: true }).click();
  await page.getByRole("button", { name: "断开连接", exact: true }).click();
  await expect(
    host.page.getByRole("button", { name: "打开 历史临时网页", exact: true }),
  ).toHaveCount(0);
  // The second identity had an empty conversation and must not add a history row.
  await history.getByRole("button", { name: /临时会话记录/ }).click();
  await expect(
    history.getByRole("button", { name: "打开 历史临时网页", exact: true }),
  ).toHaveCount(1);
  await pair(page, "草稿临时网页");
  const draftPeer = await identity(page);
  const hostInfo = await (
    await host.page.request.get(host.b.address + "/api/info")
  ).json();
  const conversationPath = encodeURIComponent(
    `direct:${hostInfo.id}:${draftPeer.id}`,
  );
  let releaseHistory!: () => void;
  let historyBlocked = false;
  const historyGate = new Promise<void>((resolve) => {
    releaseHistory = resolve;
  });
  await host.page.route(
    (url) =>
      url.pathname.includes(conversationPath) &&
      url.pathname.endsWith("/messages"),
    async (route) => {
      historyBlocked = true;
      await historyGate;
      await route.continue();
    },
  );
  await host.page
    .getByRole("button", { name: "连接 草稿临时网页", exact: true })
    .click();
  await expect(
    host.page.getByRole("heading", { name: "草稿临时网页", exact: true }),
  ).toBeVisible();
  await expect.poll(() => historyBlocked).toBe(true);
  await host.page
    .getByRole("textbox", { name: "发送文字" })
    .fill("尚未发送的临时草稿");
  // A delayed initial history read must not erase text entered after selecting the peer.
  releaseHistory();
  await expect
    .poll(async () => {
      const response = await host.page.request.get(
        host.b.address + "/api/state",
        { headers: { Authorization: `Bearer ${host.b.token}` } },
      );
      return (await response.json()).conversations.find(
        (c: any) => c.name === "草稿临时网页",
      )?.has_draft;
    })
    .toBe(true);
  await page.getByRole("button", { name: "设置", exact: true }).click();
  await page.getByRole("button", { name: "断开连接", exact: true }).click();
  const draft = history.getByRole("button", {
    name: "打开 草稿临时网页",
    exact: true,
  });
  await expect(draft).toContainText("[草稿]");
  await draft.click();
  await expect(
    host.page.getByRole("textbox", { name: "发送文字" }),
  ).toHaveValue("尚未发送的临时草稿");
  await host.page.setViewportSize({ width: 390, height: 844 });
  await host.page
    .getByRole("button", { name: "返回设备列表", exact: true })
    .click();
  expect(
    await host.page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await host.page.screenshot({
    path: ".playwright/browser-temporary-history.png",
    fullPage: true,
  });
  await host.context.close();
  await context.close();
});
