import { test, expect, type Browser } from "@playwright/test";
import { readFile } from "node:fs/promises";

async function application(browser: Browser, mode = "desktop") {
  const bootstrap = JSON.parse(
    await readFile(".playwright/bootstrap-0.json", "utf8"),
  );
  const context = await browser.newContext({
    viewport: { width: 1180, height: 820 },
  });
  await context.addInitScript(
    ({ bootstrap, mode }) => {
      const scope = window as any;
      scope.isTauri = true;
      const callbacks = new Map<number, (event: unknown) => void>();
      let sequence = 0;
      const mock = (scope.__updateMock = {
        calls: [] as { command: string; args: any }[],
        checkError: false,
        signatureValid: true,
        active: false,
        restartFails: false,
        finishDownload: () => {},
        release: {
          current_version: "0.2.0",
          version: "0.3.0",
          available: true,
          notes: "更新内容\n新的动物昵称与窗口界面",
          release_url:
            "https://github.com/whoarei/linchuan/releases/tag/v0.3.0",
          download_url:
            "https://github.com/whoarei/linchuan/releases/download/v0.3.0/Linchuan_0.3.0_android_arm64.apk",
          in_app: mode === "desktop",
        },
      });
      scope.__TAURI_INTERNALS__ = {
        transformCallback(callback: (event: unknown) => void) {
          const id = ++sequence;
          callbacks.set(id, callback);
          return id;
        },
        unregisterCallback(id: number) {
          callbacks.delete(id);
        },
        async invoke(command: string, args: any = {}) {
          mock.calls.push({ command, args });
          if (command === "bootstrap") return bootstrap;
          if (command === "desktop_window_controls") return false;
          if (command === "check_release") {
            if (mock.checkError) throw new Error("无法连接更新服务");
            return mock.release;
          }
          if (command === "plugin:updater|check")
            return {
              rid: 100,
              currentVersion: "0.2.0",
              version: "0.3.0",
              rawJson: { version: "0.3.0" },
            };
          if (command === "plugin:updater|download") {
            const callback = callbacks.get(args.onEvent.id)!;
            callback({
              index: 0,
              message: { event: "Started", data: { contentLength: 100 } },
            });
            callback({
              index: 1,
              message: { event: "Progress", data: { chunkLength: 40 } },
            });
            return new Promise<number>((resolve, reject) => {
              mock.finishDownload = () => {
                callback({
                  index: 2,
                  message: { event: "Progress", data: { chunkLength: 60 } },
                });
                callback({ index: 3, message: { event: "Finished" } });
                callback({ index: 4, end: true });
                if (mock.signatureValid) resolve(200);
                else reject(new Error("Signature mismatch"));
              };
            });
          }
          if (command === "prepare_update" && mock.active)
            throw new Error("有文件正在准备或传输，请完成后再安装更新");
          if (command === "plugin:process|restart" && mock.restartFails)
            throw new Error("Restart unavailable");
        },
      };
    },
    { bootstrap, mode },
  );
  const page = await context.newPage();
  await page.goto(bootstrap.address);
  await page.getByRole("button", { name: "关于", exact: true }).click();
  return { page, context };
}

test("signed desktop update downloads in the background, checks active transfers, installs once and retries restart", async ({
  browser,
}) => {
  const { page, context } = await application(browser);
  await page.getByRole("button", { name: "检查更新", exact: true }).click();
  await page.getByRole("button", { name: "下载更新", exact: true }).click();
  await expect(page.getByText("正在下载 40%", { exact: true })).toBeVisible();
  await expect(
    page.getByRole("button", { name: "安装并重启", exact: true }),
  ).toHaveCount(0);
  await page.getByRole("button", { name: "设备", exact: true }).click();
  await page.getByRole("button", { name: "关于", exact: true }).click();
  await expect(page.getByText("正在下载 40%", { exact: true })).toBeVisible();
  await page.evaluate(() => (window as any).__updateMock.finishDownload());
  await expect(
    page.getByText("更新包已下载并通过签名校验，安装后将重启应用", {
      exact: true,
    }),
  ).toBeVisible();
  await page.evaluate(() => {
    (window as any).__updateMock.active = true;
  });
  await page.getByRole("button", { name: "安装并重启", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("有文件正在准备或传输");
  expect(
    await page.evaluate(() =>
      (window as any).__updateMock.calls.some(
        (c: any) => c.command === "plugin:updater|install",
      ),
    ),
  ).toBe(false);
  await page.evaluate(() => {
    const state = (window as any).__updateMock;
    state.active = false;
    state.restartFails = true;
  });
  await page.getByRole("button", { name: "安装并重启", exact: true }).click();
  await expect(
    page.getByText("更新已安装，重启应用后生效", { exact: true }),
  ).toBeVisible();
  await expect(page.getByRole("alert")).toContainText("请手动重启应用");
  await page.evaluate(() => {
    (window as any).__updateMock.restartFails = false;
  });
  await page.getByRole("button", { name: "重启应用", exact: true }).click();
  expect(
    await page.evaluate(
      () =>
        (window as any).__updateMock.calls.filter(
          (c: any) => c.command === "plugin:updater|install",
        ).length,
    ),
  ).toBe(1);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__updateMock.calls.filter(
            (c: any) => c.command === "plugin:process|restart",
          ).length,
      ),
    )
    .toBe(2);
  await page.setViewportSize({ width: 360, height: 620 });
  expect(
    await page.locator(".about-page").evaluate((el) => {
      el.scrollTop = 0;
      return (
        el.querySelector(".brand-icon")!.getBoundingClientRect().top >=
        el.getBoundingClientRect().top
      );
    }),
  ).toBe(true);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: ".playwright/update-installed.png",
    fullPage: true,
  });
  await context.close();
});

test("update network failures can retry and invalid signatures never enable installation", async ({
  browser,
}) => {
  const { page, context } = await application(browser);
  await page.evaluate(() => {
    (window as any).__updateMock.checkError = true;
  });
  await page.getByRole("button", { name: "检查更新", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("无法连接更新服务");
  await expect(page.getByText("当前版本无需更新", { exact: true })).toHaveCount(
    0,
  );
  await page.evaluate(() => {
    (window as any).__updateMock.checkError = false;
    (window as any).__updateMock.signatureValid = false;
  });
  await page.getByRole("button", { name: "检查更新", exact: true }).click();
  await page.getByRole("button", { name: "下载更新", exact: true }).click();
  await page.evaluate(() => (window as any).__updateMock.finishDownload());
  await expect(page.getByRole("alert")).toContainText("下载或签名校验失败");
  await expect(
    page.getByRole("button", { name: "安装并重启", exact: true }),
  ).toHaveCount(0);
  expect(
    await page.evaluate(() =>
      (window as any).__updateMock.calls.some(
        (c: any) => c.command === "plugin:updater|install",
      ),
    ),
  ).toBe(false);
  await context.close();
});

test("Android updates open the APK in the system browser and current releases skip the desktop updater", async ({
  browser,
}) => {
  const { page, context } = await application(browser, "android");
  await page.getByRole("button", { name: "检查更新", exact: true }).click();
  await page
    .getByRole("button", { name: "下载新版安装包", exact: true })
    .click();
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as any).__updateMock.calls.some(
          (c: any) =>
            c.command === "plugin:opener|open_url" &&
            c.args.url.endsWith("android_arm64.apk"),
        ),
      ),
    )
    .toBe(true);
  expect(
    await page.evaluate(() =>
      (window as any).__updateMock.calls.some((c: any) =>
        c.command.startsWith("plugin:updater|"),
      ),
    ),
  ).toBe(false);
  await page.evaluate(() => {
    const state = (window as any).__updateMock;
    state.release.available = false;
    state.release.version = "0.2.0";
  });
  await page.getByRole("button", { name: "检查更新", exact: true }).click();
  await expect(
    page.getByText("当前版本无需更新", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "下载新版安装包", exact: true }),
  ).toHaveCount(0);
  await context.close();
});
