import { test, expect, type Browser } from "@playwright/test";
import { readFile } from "node:fs/promises";

async function desktop(
  browser: Browser,
  options = { desktop: true, bootstrapError: "" },
) {
  const bootstrap = JSON.parse(
    await readFile(".playwright/bootstrap-0.json", "utf8"),
  );
  const context = await browser.newContext({
    viewport: { width: 1180, height: 820 },
  });
  await context.addInitScript(
    ({ bootstrap, options }) => {
      const scope = window as any;
      scope.isTauri = true;
      const callbacks = new Map<number, (event: unknown) => void>();
      const listeners = new Map<number, { event: string; handler: number }>();
      let sequence = 0;
      const state = (scope.__windowMock = {
        maximized: false,
        calls: [] as { command: string; args: any }[],
        rejectNext: "",
        holdNextRead: false,
        pendingReads: [] as (() => void)[],
        emit(event: string, payload: unknown) {
          for (const [id, listener] of listeners) {
            if (listener.event === event)
              callbacks.get(listener.handler)?.({ event, id, payload });
          }
        },
        externalMaximize(value: boolean) {
          state.maximized = value;
          state.emit("tauri://resize", { width: 1180, height: 820 });
        },
      });
      scope.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
        unregisterListener: (_event: string, id: number) =>
          listeners.delete(id),
      };
      scope.__TAURI_INTERNALS__ = {
        metadata: {
          currentWindow: { label: "main" },
          currentWebview: { label: "main", windowLabel: "main" },
        },
        transformCallback(callback: (event: unknown) => void) {
          const id = ++sequence;
          callbacks.set(id, callback);
          return id;
        },
        async invoke(command: string, args: any = {}) {
          if (command === "bootstrap") {
            if (options.bootstrapError) throw new Error(options.bootstrapError);
            return bootstrap;
          }
          if (command === "desktop_window_controls") return options.desktop;
          if (command === "plugin:event|listen") {
            const id = ++sequence;
            listeners.set(id, { event: args.event, handler: args.handler });
            return id;
          }
          if (command === "plugin:event|unlisten") {
            listeners.delete(args.eventId);
            return;
          }
          if (command === "plugin:window|is_maximized") {
            const snapshot = state.maximized;
            if (state.holdNextRead) {
              state.holdNextRead = false;
              return new Promise<boolean>((resolve) =>
                state.pendingReads.push(() => resolve(snapshot)),
              );
            }
            return snapshot;
          }
          if (command === "plugin:window|is_focused") return true;
          state.calls.push({ command, args });
          if (state.rejectNext === command) {
            state.rejectNext = "";
            throw new Error("窗口接口暂时不可用");
          }
          if (command === "plugin:window|toggle_maximize")
            state.externalMaximize(!state.maximized);
        },
      };
    },
    { bootstrap, options },
  );
  const page = await context.newPage();
  await page.goto(bootstrap.address);
  return { page, context };
}

test("custom desktop controls follow native state and preserve dragging and edge resizing", async ({
  browser,
}) => {
  const { page, context } = await desktop(browser);
  const titlebar = page.getByRole("banner", { name: "窗口标题栏" });
  const maximize = page.getByRole("button", {
    name: "最大化窗口",
    exact: true,
  });
  const restore = page.getByRole("button", { name: "还原窗口", exact: true });
  await expect(titlebar).toBeVisible();
  await expect(
    page.getByRole("navigation", { name: "主要功能" }),
  ).toBeVisible();
  expect(
    await page
      .locator(".workspace")
      .evaluate((el) => el.getBoundingClientRect().top),
  ).toBe(36);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollHeight <= innerHeight,
    ),
  ).toBe(true);
  await maximize.click();
  await expect(restore).toBeVisible();
  await expect(page.locator(".window-resize-edge")).toHaveCount(0);
  await restore.click();
  await expect(maximize).toBeVisible();
  await page.evaluate(() =>
    (window as any).__windowMock.externalMaximize(true),
  );
  await expect(restore).toBeVisible();
  await page.evaluate(() =>
    (window as any).__windowMock.externalMaximize(false),
  );
  await expect(maximize).toBeVisible();

  // A late reply from an older resize must not replace the newest native state.
  await page.evaluate(() => {
    const state = (window as any).__windowMock;
    state.holdNextRead = true;
    state.externalMaximize(true);
  });
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__windowMock.pendingReads.length),
    )
    .toBe(1);
  await page.evaluate(() =>
    (window as any).__windowMock.externalMaximize(false),
  );
  await expect(maximize).toBeVisible();
  await page.evaluate(() =>
    (window as any).__windowMock.pendingReads.shift()(),
  );
  await expect(maximize).toBeVisible();

  await page.locator(".titlebar-drag").click({ button: "right" });
  expect(
    await page.evaluate(
      () =>
        (window as any).__windowMock.calls.filter(
          (call: any) => call.command === "plugin:window|start_dragging",
        ).length,
    ),
  ).toBe(0);
  await page.locator(".titlebar-drag").dblclick();
  await expect(restore).toBeVisible();
  await restore.focus();
  await page.keyboard.press("Enter");
  await expect(maximize).toBeVisible();
  await page
    .locator(".resize-southeast")
    .dispatchEvent("mousedown", { button: 0 });
  expect(
    await page.evaluate(() =>
      (window as any).__windowMock.calls.some(
        (call: any) =>
          call.command === "plugin:window|start_resize_dragging" &&
          call.args.value === "SouthEast",
      ),
    ),
  ).toBe(true);
  await page.evaluate(() =>
    (window as any).__windowMock.emit("tauri://blur", null),
  );
  await expect(titlebar).toHaveClass(/inactive/);
  await page.evaluate(() =>
    (window as any).__windowMock.emit("tauri://focus", null),
  );
  await expect(titlebar).not.toHaveClass(/inactive/);
  await page.getByRole("button", { name: "最小化窗口" }).click();
  expect(
    await page.evaluate(() =>
      (window as any).__windowMock.calls.some(
        (call: any) => call.command === "plugin:window|minimize",
      ),
    ),
  ).toBe(true);
  await page.getByRole("button", { name: "设置", exact: true }).click();
  await page.screenshot({
    path: ".playwright/custom-titlebar.png",
    fullPage: true,
  });
  await page.setViewportSize({ width: 360, height: 620 });
  expect(
    await page.evaluate(
      () =>
        document.documentElement.scrollHeight <= innerHeight &&
        document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page
    .getByRole("button", { name: "二维码与配对码", exact: true })
    .click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  expect(
    await dialog.evaluate((el) => {
      const bounds = el.getBoundingClientRect();
      return bounds.top >= 36 && bounds.bottom <= innerHeight;
    }),
  ).toBe(true);
  await expect(page.getByRole("button", { name: "关闭窗口" })).toBeVisible();
  await page.getByRole("button", { name: "关闭", exact: true }).click();
  await page.getByRole("button", { name: "关闭窗口" }).click();
  expect(
    await page.evaluate(() =>
      (window as any).__windowMock.calls.some(
        (call: any) => call.command === "plugin:window|close",
      ),
    ),
  ).toBe(true);
  await context.close();
});

test("window controls remain available during core startup failure and recover from a rejected operation", async ({
  browser,
}) => {
  const { page, context } = await desktop(browser, {
    desktop: true,
    bootstrapError: "启动服务失败",
  });
  await expect(page.getByText("启动服务失败", { exact: true })).toBeVisible();
  const maximize = page.getByRole("button", {
    name: "最大化窗口",
    exact: true,
  });
  await page.evaluate(() => {
    (window as any).__windowMock.rejectNext = "plugin:window|toggle_maximize";
  });
  await maximize.click();
  await expect(page.getByRole("status")).toContainText("窗口操作失败");
  await expect(maximize).toBeEnabled();
  await maximize.click();
  await expect(
    page.getByRole("button", { name: "还原窗口", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "关闭窗口" }).click();
  expect(
    await page.evaluate(() =>
      (window as any).__windowMock.calls.some(
        (call: any) => call.command === "plugin:window|close",
      ),
    ),
  ).toBe(true);
  await context.close();
});

test("Android native UI and normal browsers have no desktop titlebar or reserved space", async ({
  browser,
  page,
}) => {
  const android = await desktop(browser, {
    desktop: false,
    bootstrapError: "",
  });
  await expect(
    android.page.getByRole("navigation", { name: "主要功能" }),
  ).toBeVisible();
  await expect(
    android.page.getByRole("banner", { name: "窗口标题栏" }),
  ).toHaveCount(0);
  expect(
    await android.page.evaluate(
      () => getComputedStyle(document.body).paddingTop,
    ),
  ).toBe("0px");
  await android.context.close();
  const bootstrap = JSON.parse(
    await readFile(".playwright/bootstrap-0.json", "utf8"),
  );
  await page.goto(bootstrap.address);
  await expect(page.getByLabel("六位配对码")).toBeVisible();
  await expect(page.getByRole("banner", { name: "窗口标题栏" })).toHaveCount(0);
  expect(
    await page.evaluate(() => getComputedStyle(document.body).paddingTop),
  ).toBe("0px");
});
