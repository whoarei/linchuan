import { test, expect, type Browser } from "@playwright/test";
import { mkdir, readFile, writeFile } from "node:fs/promises";

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
          if (command === "receive_transfer_as") {
            if (state.cancel) {
              state.cancel = false;
              return false;
            }
            if (state.failPicker) {
              state.failPicker = false;
              throw "目标磁盘空间不足";
            }
            // The real native command accepts directly in core after picking a folder.
            const response = await fetch(
              `${boot.address}/api/transfers/${args.transferId}/decision`,
              {
                method: "POST",
                headers: {
                  Authorization: `Bearer ${boot.token}`,
                  "Content-Type": "application/json",
                },
                body: JSON.stringify({ accept: true, file_ids: args.fileIds, choices: args.choices }),
              },
            );
            if (!response.ok) {
              const error = await response.json();
              if (error.conflicts) return { directory: state.directory, conflicts: error.conflicts };
              throw new Error(error.error);
            }
            return true;
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
                t.files.filter((f: any) => !t.selected_file_ids || t.selected_file_ids.includes(f.id)).map((f: any) => ({
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

async function sendPendingFile(browser: Browser, address: string, code: string, name: string, filename = "自动保存.txt") {
  const sender = await browser.newContext();
  const page = await sender.newPage();
  await page.goto(address);
  await page.getByLabel("你的设备名称").fill(name);
  await page.getByLabel("六位配对码").fill(code);
  await page.getByRole("button", { name: "连接设备", exact: true }).click();
  await expect(page.getByRole("textbox", {name:"发送文字"})).toBeVisible();
  await page.locator("input[type=file]").setInputFiles({
    name: filename,
    mimeType: "text/plain",
    buffer: Buffer.from("saved bytes"),
  });
  await page
    .getByRole("textbox", { name: "发送文字" })
    .fill("自动保存到固定目录");
  await page.getByRole("button", { name: "发送", exact: true }).click();
  return sender;
}

test("save as keeps an offer pending on cancellation or a storage error", async ({ browser }) => {
  const host = await client(browser);
  const name = "另存为测试浏览器";
  const sender = await sendPendingFile(browser, host.boot.address, host.boot.pairing_code, name, "另存为.txt");
  await host.page.getByRole("button", { name: `打开 ${name}`, exact: true }).click();
  const actions = host.page.getByRole("group", { name: "接收文件" });
  const saveAs = actions.getByRole("button", { name: "另存为", exact: true });
  await expect(saveAs.locator("svg")).toBeVisible();
  await expect(saveAs).toHaveText("");
  await expect(actions.getByRole("button", { name: "接收", exact: true })).toBeVisible();
  await expect(actions.getByRole("button", { name: "拒绝", exact: true })).toBeVisible();
  await host.page.evaluate(() => ((window as any).__receiving.cancel = true));
  await saveAs.click();
  await expect(saveAs).toBeEnabled();
  await expect(actions).toContainText("待接收");
  await host.page.setViewportSize({ width: 390, height: 844 });
  await expect(saveAs).toBeVisible();
  expect(await host.page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await host.page.evaluate(() => ((window as any).__receiving.failPicker = true));
  await saveAs.click();
  await expect(host.page.locator(".toast")).toContainText("目标磁盘空间不足");
  await expect(actions).toContainText("待接收");
  const folder = ".playwright/device-0/received";
  await mkdir(folder, { recursive: true });
  await writeFile(`${folder}/另存为.txt`, "existing");
  await saveAs.click();
  const conflict = host.page.getByRole("dialog", { name: "接收文件时遇到重名" });
  await expect(conflict.getByRole("button", { name: "跳过", exact: true })).toBeEnabled();
  await conflict.getByRole("button", { name: "保留两份" }).click();
  await expect(actions).toHaveCount(0);
  await expect(host.page.locator(".message-attachments")).toContainText("已保存");
  expect(await readFile(`${folder}/另存为.txt`, "utf8")).toBe("existing");
  expect(await readFile(`${folder}/另存为 (1).txt`, "utf8")).toBe("saved bytes");
  expect(await host.page.evaluate(() => (window as any).__receiving.calls
    .filter((c: any) => c.command === "receive_transfer_as").at(-1).args.directory)).toBeTruthy();
  await sender.close();
  await host.context.close();
});

test("selected files upload smallest first and both sides display transfer speed", async ({ browser }) => {
  const host = await client(browser, true);
  const sender = await browser.newContext();
  const page = await sender.newPage();
  await page.goto(host.boot.address);
  await page.getByLabel("你的设备名称").fill("多文件浏览器");
  await page.getByLabel("六位配对码").fill(host.boot.pairing_code);
  await page.getByRole("button", { name: "连接设备", exact: true }).click();
  await expect(page.getByRole("textbox", { name: "发送文字" })).toBeVisible();

  const cdp = await sender.newCDPSession(page);
  await cdp.send("Network.emulateNetworkConditions", {
    offline: false,
    latency: 0,
    downloadThroughput: -1,
    uploadThroughput: 128 * 1024,
  });
  const size = 768 * 1024;
  const uploads: string[] = [];
  page.on("request", (request) => {
    if (request.method() === "PUT" && request.url().endsWith("/upload")) uploads.push(request.url());
  });
  await page.locator("input[type=file]").setInputFiles([
    { name: "update.img", mimeType: "application/octet-stream", buffer: Buffer.alloc(size) },
    { name: "skip.bin", mimeType: "application/octet-stream", buffer: Buffer.alloc(1024) },
    { name: "update.sha256", mimeType: "text/plain", buffer: Buffer.from("checksum") },
  ]);
  await page.getByRole("button", { name: "发送", exact: true }).click();
  await host.page.getByRole("button", { name: "打开 多文件浏览器", exact: true }).click();
  const attachments = host.page.locator(".message-attachments");
  await attachments.getByRole("checkbox", { name: "接收 skip.bin" }).uncheck();
  await expect(attachments).toContainText("已选 2/3 个文件");
  await attachments.getByRole("button", { name: "另存为", exact: true }).click();

  const speed = /[1-9]\d*(?:\.\d+)? (?:B|KB|MB|GB)\/s/;
  await Promise.all([
    expect(attachments.locator(".attachment-transfer")).toContainText(speed),
    expect(page.locator(".delivery-details")).toContainText(speed),
  ]);
  expect(await readFile(".playwright/device-0/received/update.sha256", "utf8")).toBe("checksum");
  await expect(attachments.locator(".attachment-card").filter({ hasText: "update.img" })).toContainText("已保存", { timeout: 12_000 });
  await expect(attachments.locator(".attachment-card").filter({ hasText: "skip.bin" })).toContainText("未选择接收");
  await expect(page.locator(".delivery-summary")).toContainText("已传送 2/3 个文件");
  const { transfers } = await (await host.page.request.get(`${host.boot.address}/api/state`, {
    headers: { Authorization: `Bearer ${host.boot.token}` },
  })).json();
  const transfer = transfers.find((t: any) => t.files.some((f: any) => f.name === "update.sha256"));
  expect(transfer.total).toBe(size + Buffer.byteLength("checksum"));
  const checksum = transfer.files.find((f: any) => f.name === "update.sha256");
  const image = transfer.files.find((f: any) => f.name === "update.img");
  expect(uploads).toEqual([checksum, image].map((file) => `${host.boot.address}/api/transfers/${transfer.id}/files/${file.id}/upload`));
  expect(await host.page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await sender.close();
  await host.context.close();
});

test("name conflicts stay pending until the recipient confirms a mixed batch", async ({ browser }) => {
  const host = await client(browser, true);
  const folder = ".playwright/device-0/received";
  const names = ["覆盖.txt", "保留.txt", "跳过.txt"];
  await mkdir(folder, { recursive: true });
  await Promise.all([...names, "保留 (1).txt"].map(name => writeFile(`${folder}/${name}`, "old")));
  const sender = await browser.newContext();
  const page = await sender.newPage();
  await page.goto(host.boot.address);
  await page.getByLabel("你的设备名称").fill("重名测试浏览器");
  await page.getByLabel("六位配对码").fill(host.boot.pairing_code);
  await page.getByRole("button", { name: "连接设备", exact: true }).click();
  await expect(page.getByRole("textbox", { name: "发送文字" })).toBeVisible();
  await page.locator("input[type=file]").setInputFiles([...names, "正常.txt"].map(name => ({
    name, mimeType: "text/plain", buffer: Buffer.from("new-" + name),
  })));
  await page.getByRole("button", { name: "发送", exact: true }).click();
  await host.page.getByRole("button", { name: "打开 重名测试浏览器", exact: true }).click();
  const accept = host.page.getByRole("button", { name: "接收", exact: true });
  await accept.click();
  const dialog = host.page.getByRole("dialog", { name: "接收文件时遇到重名" });
  await expect(dialog.getByRole("button", { name: "继续接收" })).toBeDisabled();
  await dialog.getByRole("button", { name: "返回" }).click();
  await expect(accept).toBeEnabled();
  expect(await readFile(`${folder}/覆盖.txt`, "utf8")).toBe("old");
  await expect(readFile(`${folder}/正常.txt`)).rejects.toThrow();
  await accept.click();
  await dialog.getByLabel("批量处理同名文件").selectOption("keep_both");
  await dialog.getByLabel("覆盖.txt 的处理方式").selectOption("replace");
  await dialog.getByLabel("跳过.txt 的处理方式").selectOption("skip");
  await dialog.getByRole("button", { name: "继续接收" }).click();
  await expect.poll(() => readFile(`${folder}/覆盖.txt`, "utf8")).toBe("new-覆盖.txt");
  await expect.poll(() => readFile(`${folder}/正常.txt`, "utf8").catch(() => "")).toBe("new-正常.txt");
  await expect.poll(() => readFile(`${folder}/保留 (2).txt`, "utf8").catch(() => "")).toBe("new-保留.txt");
  for (const name of ["保留.txt", "保留 (1).txt", "跳过.txt"]) {
    expect(await readFile(`${folder}/${name}`, "utf8")).toBe("old");
  }
  await expect(host.page.locator(".attachment-card").filter({ hasText: "跳过.txt" })).toContainText("未选择接收");
  await sender.close();
  await host.context.close();
});

test("native saved files open through IPC after cache cleanup and failures can retry without browser download", async ({
  browser,
}) => {
  const host = await client(browser);
  const sender = await sendPendingFile(browser, host.boot.address, host.boot.pairing_code, "固定目录测试浏览器");
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
