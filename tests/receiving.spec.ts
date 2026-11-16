import { test, expect, type Browser } from "@playwright/test";
import { readFile } from "node:fs/promises";

async function client(browser: Browser, mobile = false) {
  const boot = JSON.parse(
    await readFile(".playwright/bootstrap-0.json", "utf8"),
  );
  const context = await browser.newContext({
    viewport: mobile
      ? { width: 390, height: 844 }
      : { width: 1180, height: 820 },
  });
  await context.addInitScript(
    ({ boot, mobile }) => {
      const scope = window as any;
      scope.isTauri = true;
      const state = (scope.__receiving = {
        directory: mobile ? "下载/邻传" : "/home/user/Downloads/邻传",
        status: "saved",
        calls: [] as { command: string; args: any }[],
        cancel: false,
        failPicker: false,
        fileLocation: "/home/user/Downloads/邻传/自动保存.txt",
      });
      scope.__TAURI_INTERNALS__ = {
        async invoke(command: string, args: any = {}) {
          if (command === "bootstrap") return boot;
          if (command === "desktop_window_controls") return false;
          state.calls.push({ command, args });
          if (command === "receive_settings")
            return { directory: state.directory, label: state.directory };
          if (command === "choose_receive_directory") {
            if (state.failPicker) {
              state.failPicker = false;
              throw new Error("目录不可写");
            }
            if (!state.cancel) state.directory = "自定义接收目录";
            state.cancel = false;
            return { directory: state.directory, label: state.directory };
          }
          if (command === "reset_receive_directory") {
            state.directory = mobile
              ? "下载/邻传"
              : "/home/user/Downloads/邻传";
            return { directory: state.directory, label: state.directory };
          }
          if (command === "save_received_file") state.status = "saved";
          if (command === "relocate_received_file") {
            if (state.cancel) {
              state.cancel = false;
              return null;
            }
            if (state.failPicker) {
              state.failPicker = false;
              throw new Error("模拟移动失败，原文件和记录保留");
            }
            if (args.moveFile) state.fileLocation = "/other/自动保存.txt";
            return { name: "自动保存.txt", location: "/other/自动保存.txt" };
          }
          if (command === "received_files") {
            const response = await fetch(boot.address + "/api/state", {
              headers: { Authorization: `Bearer ${boot.token}` },
            });
            const app = await response.json();
            return app.transfers
              .filter(
                (t: any) =>
                  args.messages.includes(t.message_id) &&
                  t.status === "completed" &&
                  t.recipient_id === app.me.id,
              )
              .flatMap((t: any) =>
                t.files.map((f: any) => ({
                  message_id: t.message_id,
                  file_id: f.id,
                  name: f.name,
                  status: state.status,
                  error: state.status === "failed" ? "磁盘空间不足" : null,
                  location: state.fileLocation,
                  directory: state.directory,
                })),
              );
          }
        },
      };
    },
    { boot, mobile },
  );
  const page = await context.newPage();
  await page.goto(boot.address);
  await expect(
    page.getByRole("navigation", { name: "主要功能" }),
  ).toBeVisible();
  return { page, context, boot };
}

for (const mobile of [false, true]) {
  test(`receive directory can be changed, cancelled and reset on ${mobile ? "mobile" : "desktop"}`, async ({
    browser,
  }) => {
    const { page, context } = await client(browser, mobile);
    await page.getByRole("button", { name: "设置", exact: true }).click();
    const card = page.getByRole("region", { name: "接收目录" });
    await expect(card).toContainText(
      mobile ? "下载/邻传" : "/home/user/Downloads/邻传",
    );
    await card.getByRole("button", { name: "更改目录" }).click();
    await expect(card).toContainText("自定义接收目录");
    await page.evaluate(() => ((window as any).__receiving.cancel = true));
    await card.getByRole("button", { name: "更改目录" }).click();
    await expect(card).toContainText("自定义接收目录");
    await card.getByRole("button", { name: "打开目录", exact: true }).click();
    await card.getByRole("button", { name: "恢复默认" }).click();
    await expect(card).toContainText(
      mobile ? "下载/邻传" : "/home/user/Downloads/邻传",
    );
    await page.evaluate(() => ((window as any).__receiving.failPicker = true));
    await card.getByRole("button", { name: "更改目录" }).click();
    await expect(page.locator(".toast")).toContainText("目录不可写");
    await expect(card.getByRole("button", { name: "更改目录" })).toBeEnabled();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    expect(
      await page.evaluate(() =>
        (window as any).__receiving.calls.some(
          (c: any) => c.command === "open_receive_directory",
        ),
      ),
    ).toBe(true);
    await context.close();
  });
}

test("native saved files open through IPC after cache cleanup and failures can retry without browser download", async ({
  browser,
}) => {
  const host = await client(browser);
  const sender = await browser.newContext();
  const page = await sender.newPage();
  await page.goto(host.boot.address);
  await page.getByLabel("你的设备名称").fill("固定目录测试浏览器");
  await page.getByLabel("六位配对码").fill(host.boot.pairing_code);
  await page.getByRole("button", { name: "连接设备", exact: true }).click();
  await expect(page.getByRole("textbox", {name:"发送文字"})).toBeVisible();
  await page.locator("input[type=file]").setInputFiles({
    name: "自动保存.txt",
    mimeType: "text/plain",
    buffer: Buffer.from("saved bytes"),
  });
  await page
    .getByRole("textbox", { name: "发送文字" })
    .fill("自动保存到固定目录");
  await page.getByRole("button", { name: "发送", exact: true }).click();
  await host.page
    .getByRole("button", { name: "打开 固定目录测试浏览器", exact: true })
    .click();
  const message = host.page
    .locator(".message")
    .filter({ hasText: "自动保存到固定目录" });
  await host.page.evaluate(
    () => ((window as any).__receiving.status = "failed"),
  );
  await message.getByRole("button", { name: "接收", exact: true }).click();
  await expect(message).toContainText("磁盘空间不足");
  let tickets = 0;
  host.page.on("request", (r) => {
    if (r.url().endsWith("/ticket")) tickets++;
  });
  await message.getByRole("button", { name: "重新保存 自动保存.txt" }).click();
  await expect(message).toContainText("已保存");
  const savedLocation = message.locator(".file-label small");
  await expect(savedLocation).toHaveAttribute(
    "title",
    "/home/user/Downloads/邻传/自动保存.txt",
  );
  await message
    .getByRole("button", { name: "复制到其他位置 自动保存.txt" })
    .click();
  await expect(host.page.getByRole("status")).toHaveText(
    "已复制，原文件位置保持不变",
  );
  await expect(savedLocation).toHaveAttribute(
    "title",
    "/home/user/Downloads/邻传/自动保存.txt",
  );
  await host.page.evaluate(() => ((window as any).__receiving.cancel = true));
  await message
    .getByRole("button", { name: "移动到其他位置 自动保存.txt" })
    .click();
  await expect(savedLocation).toHaveAttribute(
    "title",
    "/home/user/Downloads/邻传/自动保存.txt",
  );
  await host.page.evaluate(
    () => ((window as any).__receiving.failPicker = true),
  );
  await message
    .getByRole("button", { name: "移动到其他位置 自动保存.txt" })
    .click();
  await expect(host.page.getByRole("status")).toContainText("原文件和记录保留");
  await expect(savedLocation).toHaveAttribute(
    "title",
    "/home/user/Downloads/邻传/自动保存.txt",
  );
  await message
    .getByRole("button", { name: "移动到其他位置 自动保存.txt" })
    .click();
  await expect(host.page.getByRole("status")).toHaveText(
    "已移动，文件位置已更新",
  );
  await expect(savedLocation).toHaveAttribute("title", "/other/自动保存.txt");
  await expect(message.locator(".attachment-actions").getByRole("button")).toHaveCount(4);
  await message.locator(".message-attachments").screenshot({ path: ".playwright/file-actions-desktop.png" });
  await host.page.setViewportSize({ width: 390, height: 844 });
  await expect(message.getByRole("button", { name: "复制到其他位置 自动保存.txt" })).toBeVisible();
  await expect(message.getByRole("button", { name: "移动到其他位置 自动保存.txt" })).toBeVisible();
  expect(await host.page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await message.locator(".message-attachments").screenshot({ path: ".playwright/file-actions-mobile.png" });
  await host.page.setViewportSize({ width: 1180, height: 820 });
  await message
    .getByRole("button", { name: "打开文件 自动保存.txt", exact: true })
    .click();
  await message
    .getByRole("button", { name: "打开所在文件夹 自动保存.txt", exact: true })
    .click();
  expect(tickets).toBe(0);
  await host.page.evaluate(async (boot) => {
    await fetch(boot.address + "/api/storage/clear", {
      method: "POST",
      headers: {
        Authorization: `Bearer ${boot.token}`,
        "Content-Type": "application/json",
      },
      body: "{}",
    });
  }, host.boot);
  await expect(
    message.getByRole("button", { name: "保存副本 自动保存.txt" }),
  ).toHaveCount(0);
  await expect(
    message.getByRole("button", { name: "打开文件 自动保存.txt", exact: true }),
  ).toBeVisible();
  await expect(message.locator(".attachment-card")).not.toHaveClass(/expired/);
  await message
    .getByRole("button", { name: "打开文件 自动保存.txt", exact: true })
    .click();
  const calls = await host.page.evaluate(
    () => (window as any).__receiving.calls,
  );
  expect(
    calls.filter((c: any) => c.command === "save_received_file"),
  ).toHaveLength(1);
  expect(
    calls
      .filter((c: any) => c.command === "open_received_file")
      .map((c: any) => c.args.folder),
  ).toEqual([false, true, false]);
  await sender.close();
  await host.context.close();
});
