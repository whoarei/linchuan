import { test, expect, type Browser, type Page } from "@playwright/test";
import { readFile } from "node:fs/promises";
async function boot(index = 0) {
  return JSON.parse(
    await readFile(`.playwright/bootstrap-${index}.json`, "utf8"),
  );
}
async function nicknames() {
  const words = JSON.parse(await readFile("shared/device-names.json", "utf8"));
  return new Set<string>(
    words.moods.flatMap((mood: string) =>
      words.animals.map((animal: string) => mood + animal),
    ),
  );
}
async function me(page: Page) {
  return page.evaluate(async () => {
    const response = await fetch("/api/state", {
      headers: {
        Authorization: `Bearer ${sessionStorage.getItem("linchuan-token")}`,
      },
    });
    return (await response.json()).me;
  });
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
  await expect(page.getByRole("textbox", {name:"发送文字"})).toBeVisible();
  await page
    .getByRole("textbox", { name: "发送文字" })
    .fill("来自浏览器的一段文字");
  const bytes = Buffer.from("邻传原始内容\n图片、文字与文件");
  await page.locator("input[type=file]").setInputFiles({
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
test("native group members receive independently while browser pages stay host-only", async ({browser}) => {
    const host=await owner(browser);const a=await owner(browser,1);const b=await owner(browser,2);
    await addPeer(host.page,a.b.address);await addPeer(host.page,b.b.address);
    const aname=(await me(a.page)).name;const bname=(await me(b.page)).name;
    await host.page.getByRole("button",{name:"添加",exact:true}).click();
    await host.page.getByRole("button",{name:"新建群聊",exact:true}).click();
    await host.page.getByLabel("群名称").fill("客户端资料群");
    await host.page.getByRole("checkbox",{name:new RegExp(aname)}).check();
    await host.page.getByRole("checkbox",{name:new RegExp(bname)}).check();
    await host.page.getByRole("button",{name:"创建群聊",exact:true}).click();
    for(const member of [a,b]) {
      await member.page.getByRole("button",{name:"打开 客户端资料群",exact:true}).click();
      await member.page.getByRole("button",{name:"加入群聊",exact:true}).click();
    }
    await host.page.getByRole("textbox",{name:"发送文字"}).fill("群文件只发送一条消息");
    await host.page.locator("input[type=file]").setInputFiles({name:"群资料.txt",mimeType:"text/plain",buffer:Buffer.from("group bytes")});
    await host.page.getByRole("button",{name:"发送",exact:true}).click();
    const card=host.page.locator(".message").filter({hasText:"群文件只发送一条消息"});
    await a.page.locator(".message").filter({hasText:"群文件只发送一条消息"}).getByRole("button",{name:"接收",exact:true}).click();
    await expect(b.page.locator(".message").filter({hasText:"群文件只发送一条消息"}).getByRole("button",{name:"接收",exact:true})).toBeVisible();
    await b.page.locator(".message").filter({hasText:"群文件只发送一条消息"}).getByRole("button",{name:"接收",exact:true}).click();
    await expect(card.getByText("已送达",{exact:true})).toBeVisible();
    await host.page.screenshot({path:".playwright/desktop.png",fullPage:true});
    await Promise.all([host.context.close(),a.context.close(),b.context.close()]);
  });

test("settings exposes device rename; drafts and history survive a page reload", async ({
  browser,
}) => {
  const host = await owner(browser);
  await host.page
    .getByRole("button", { name: "打开 客户端资料群", exact: true })
    .click();
  await host.page
    .getByRole("textbox", { name: "发送文字" })
    .fill("还没有发送的草稿");
  await host.page.getByRole("button", { name: "设置", exact: true }).click();
  await host.page
    .getByRole("button", { name: "换个名字", exact: true })
    .click();
  expect(
    (await nicknames()).has(
      await host.page.getByLabel("设备名称", { exact: true }).inputValue(),
    ),
  ).toBe(true);
  await host.page
    .getByLabel("设备名称", { exact: true })
    .fill("重命名后的电脑");
  await host.page
    .getByRole("button", { name: "保存名称", exact: true })
    .click();
  await expect(host.page.getByRole("status")).toHaveText("设备名称已保存");
  await host.page.reload();
  await host.page
    .getByRole("button", { name: "打开 客户端资料群", exact: true })
    .click();
  await expect(
    host.page.getByRole("textbox", { name: "发送文字" }),
  ).toHaveValue("还没有发送的草稿");
  await expect(
    host.page.locator(".message").filter({ hasText: "群文件只发送一条消息" }),
  ).toHaveCount(1);
  await host.page.getByRole("button", { name: "关于", exact: true }).click();
  await expect(
    host.page.getByRole("heading", { name: "邻传 · Linchuan" }),
  ).toBeVisible();
  const version = JSON.parse(await readFile("package.json", "utf8")).version;
  await expect(host.page.getByText(version, { exact: true })).toBeVisible();
  await host.context.close();
});
test("mobile browser opens its host directly without device navigation or horizontal overflow", async ({
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
  await expect(page.getByRole("textbox", {name:"发送文字"})).toBeVisible();
  await expect(page.getByRole("textbox", { name: "发送文字" })).toBeVisible();
  await expect(page.getByRole("navigation", { name: "主要功能" })).toBeVisible();
  await expect(page.locator(".conversation-panel")).toHaveCount(0);
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
  await expect(page.getByRole("button", { name: "返回设备列表" })).toHaveCount(0);
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

test("browser names are automatic, editable and remembered without changing identity", async ({
  browser,
}) => {
  const context = await browser.newContext({
    userAgent:
      "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/131.0.0.0 Safari/537.36 Edg/131.0.0.0",
  });
  const page = await context.newPage();
  const b = await boot();
  await page.goto(b.address);
  const field = page.getByLabel("你的设备名称");
  await expect(field).not.toBeEmpty();
  const automatic = await field.inputValue();
  const choices = await nicknames();
  expect(choices.has(automatic)).toBe(true);
  await page.reload();
  await expect(field).toHaveValue(automatic);
  await field.fill("");
  await page.getByLabel("六位配对码").fill(b.pairing_code);
  await page.getByRole("button", { name: "连接设备", exact: true }).click();
  await expect(
    page.getByRole("navigation", { name: "主要功能" }),
  ).toBeVisible();
  const identity = (await me(page)).id;
  expect((await me(page)).platform).toBe("Windows · Edge");
  await expect(page.locator(".list-footer")).toHaveCount(0);
  await page.getByRole("button", { name: "设置", exact: true }).click();
  await expect(page.getByLabel("设备名称", { exact: true })).toHaveValue(
    automatic,
  );
  await page.getByLabel("设备名称", { exact: true }).fill("我的 Edge 网页");
  await page.getByRole("button", { name: "保存名称", exact: true }).click();
  await expect(page.getByRole("status")).toHaveText("设备名称已保存");
  await page.reload();
  await page.getByRole("button", { name: "设置", exact: true }).click();
  await expect(page.getByLabel("设备名称", { exact: true })).toHaveValue(
    "我的 Edge 网页",
  );
  await page.setViewportSize({ width: 390, height: 844 });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: ".playwright/browser-name-settings.png",
    fullPage: true,
  });
  const renamedIdentity = (await me(page)).id;
  expect(renamedIdentity).toBe(identity);
  await page.getByRole("button", { name: "换个名字", exact: true }).click();
  const generated = await page
    .getByLabel("设备名称", { exact: true })
    .inputValue();
  expect(choices.has(generated)).toBe(true);
  expect((await me(page)).name).toBe("我的 Edge 网页");
  await page.getByRole("button", { name: "换个名字", exact: true }).click();
  const next = await page.getByLabel("设备名称", { exact: true }).inputValue();
  expect(next).not.toBe(generated);
  expect(choices.has(next)).toBe(true);
  await page.getByRole("button", { name: "保存名称", exact: true }).click();
  await expect(page.getByRole("status")).toHaveText("设备名称已保存");
  await page.reload();
  expect((await me(page)).id).toBe(identity);
  expect((await me(page)).name).toBe(next);
  await page.getByRole("button", { name: "设置", exact: true }).click();
  await expect(page.getByLabel("设备名称", { exact: true })).toHaveValue(next);
  await page.getByRole("button", { name: "断开连接", exact: true }).click();
  await expect(field).toHaveValue(next);
  await page.reload();
  await expect(field).toHaveValue(next);
  await context.close();
});

test("matching nicknames are distinguished across nearby devices and conversations", async ({
  browser,
}) => {
  const host = await owner(browser);
  const a = await browser.newContext();
  const b = await browser.newContext();
  const first = await a.newPage();
  const second = await b.newPage();
  await pair(first, "摸鱼海獭");
  await pair(second, "摸鱼海獭");
  const one = await me(first);
  const two = await me(second);
  let suffixLength = 4;
  while (one.id.slice(-suffixLength) === two.id.slice(-suffixLength))
    suffixLength += 4;
  const oneLabel = `摸鱼海獭 · ${one.id.slice(-suffixLength).toUpperCase()}`;
  const twoLabel = `摸鱼海獭 · ${two.id.slice(-suffixLength).toUpperCase()}`;
  await expect(
    host.page.getByRole("button", { name: `连接 ${oneLabel}`, exact: true }),
  ).toBeVisible();
  await expect(
    host.page.getByRole("button", { name: `连接 ${twoLabel}`, exact: true }),
  ).toBeVisible();
  await host.page
    .getByRole("button", { name: `连接 ${oneLabel}`, exact: true })
    .click();
  await expect(
    host.page.getByRole("heading", { name: oneLabel, exact: true }),
  ).toBeVisible();
  await expect(
    host.page.getByRole("button", { name: `打开 ${oneLabel}`, exact: true }),
  ).toBeVisible();
  await expect(
    host.page.getByRole("button", { name: `连接 ${twoLabel}`, exact: true }),
  ).toBeVisible();
  await host.page.screenshot({
    path: ".playwright/nickname-collision.png",
    fullPage: true,
  });
  await first.getByRole("button", { name: "设置", exact: true }).click();
  await first.getByLabel("设备名称", { exact: true }).fill("好奇水豚");
  await first.getByRole("button", { name: "保存名称", exact: true }).click();
  await expect(
    host.page.getByRole("button", { name: "打开 好奇水豚", exact: true }),
  ).toBeVisible();
  await expect(
    host.page.getByRole("button", { name: "连接 摸鱼海獭", exact: true }),
  ).toBeVisible();
  expect((await me(first)).id).toBe(one.id);
  expect((await me(second)).id).toBe(two.id);
  await host.context.close();
  await a.close();
  await b.close();
});

test("existing browser names are preserved when animal nicknames are introduced", async ({
  browser,
}) => {
  for (const preferred of ["", "我的旧名字"]) {
    const context = await browser.newContext();
    await context.addInitScript((name) => {
      if (!localStorage.getItem("linchuan-browser-auto-name"))
        localStorage.setItem(
          "linchuan-browser-auto-name",
          "Windows · Edge · A7F2",
        );
      if (name) localStorage.setItem("linchuan-browser-name", name);
    }, preferred);
    const page = await context.newPage();
    await page.goto((await boot()).address);
    await expect(page.getByLabel("你的设备名称")).toHaveValue(
      preferred || "Windows · Edge · A7F2",
    );
    await page.reload();
    await expect(page.getByLabel("你的设备名称")).toHaveValue(
      preferred || "Windows · Edge · A7F2",
    );
    await context.close();
  }
});
