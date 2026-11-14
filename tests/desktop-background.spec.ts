import { test, expect, type Browser } from "@playwright/test";
import { readFile } from "node:fs/promises";

async function client(
  browser: Browser,
  mode: "desktop" | "android" | "browser" = "desktop",
  tray = true,
) {
  const bootstrap = JSON.parse(
    await readFile(".playwright/bootstrap-0.json", "utf8"),
  );
  const context = await browser.newContext();
  await context.addInitScript(
    ({ bootstrap, mode, tray }) => {
      const scope = window as any;
      const preferences = JSON.parse(
        localStorage.getItem("desktop-test-preferences") ||
          '{"autostart":false,"close_to_tray":true}',
      );
      const state = (scope.__background = {
        preferences,
        calls: [] as { command: string; args: any }[],
        rejectNext: "",
        holdRead: false,
        pendingReads: [] as (() => void)[],
      });
      if (mode === "browser") {
        sessionStorage.setItem("linchuan-token", bootstrap.token);
        return;
      }
      scope.isTauri = true;
      scope.__TAURI_INTERNALS__ = {
        async invoke(command: string, args: any = {}) {
          if (command === "bootstrap") return bootstrap;
          if (command === "desktop_window_controls") return false;
          if (command === "receive_settings")
            return { directory: "/Downloads/邻传", label: "/Downloads/邻传" };
          if (command === "received_files") return [];
          state.calls.push({ command, args });
          const settings = () => ({
            ...state.preferences,
            supported: mode === "desktop",
            tray_available: tray,
          });
          if (command === "desktop_settings") {
            const value = settings();
            if (state.holdRead) {
              state.holdRead = false;
              return new Promise((resolve) =>
                state.pendingReads.push(() => resolve(value)),
              );
            }
            return value;
          }
          if (command === state.rejectNext) {
            state.rejectNext = "";
            throw new Error("系统拒绝修改开机启动");
          }
          if (command === "desktop_set_autostart")
            state.preferences.autostart = args.enabled;
          if (command === "desktop_set_close_to_tray")
            state.preferences.close_to_tray = args.enabled;
          if (command.startsWith("desktop_set_")) {
            localStorage.setItem(
              "desktop-test-preferences",
              JSON.stringify(state.preferences),
            );
            return settings();
          }
        },
      };
    },
    { bootstrap, mode, tray },
  );
  const page = await context.newPage();
  await page.goto(bootstrap.address);
  await page.getByRole("button", { name: "设置", exact: true }).click();
  return { page, context };
}

test("desktop startup and close preferences persist, recover from OS rejection, and ignore stale reads", async ({
  browser,
}) => {
  const { page, context } = await client(browser);
  const startup = page.getByRole("switch", { name: "开机启动", exact: true });
  const background = page.getByRole("switch", {
    name: "关闭窗口后继续运行",
    exact: true,
  });
  await expect(startup).not.toBeChecked();
  await expect(background).toBeChecked();
  await page.evaluate(
    () => ((window as any).__background.rejectNext = "desktop_set_autostart"),
  );
  await startup.click();
  await expect(page.getByRole("status")).toContainText("系统拒绝修改开机启动");
  await expect(startup).not.toBeChecked();
  await expect(startup).toBeEnabled();
  // Read the old OS state, then change it before the delayed read returns.
  await page.evaluate(() => ((window as any).__background.holdRead = true));
  await page.getByRole("button", { name: "设置", exact: true }).click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__background.pendingReads.length),
    )
    .toBe(1);
  await startup.check();
  await expect(startup).toBeChecked();
  await page.evaluate(() =>
    (window as any).__background.pendingReads.shift()(),
  );
  await expect(startup).toBeChecked();
  await background.uncheck();
  await expect(background).not.toBeChecked();
  await page.reload();
  await page.getByRole("button", { name: "设置", exact: true }).click();
  await expect(startup).toBeChecked();
  await expect(background).not.toBeChecked();
  await page.getByRole("button", { name: "退出邻传", exact: true }).click();
  expect(
    await page.evaluate(
      () =>
        (window as any).__background.calls.filter(
          (call: any) => call.command === "desktop_quit",
        ).length,
    ),
  ).toBe(1);
  await context.close();
});

test("unavailable tray exposes a visible exit fallback and disables hiding the window", async ({
  browser,
}) => {
  const { page, context } = await client(browser, "desktop", false);
  await expect(
    page.getByText("当前托盘不可用，关闭窗口会退出应用。", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("switch", { name: "关闭窗口后继续运行", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByRole("button", { name: "退出邻传", exact: true }),
  ).toBeVisible();
  await context.close();
});

for (const mode of ["android", "browser"] as const) {
  test(`${mode} does not expose host startup or tray settings`, async ({
    browser,
  }) => {
    const { page, context } = await client(browser, mode);
    await expect(page.getByRole("region", { name: "启动与后台" })).toHaveCount(
      0,
    );
    expect(
      await page.evaluate(() =>
        (window as any).__background.calls.some(
          (call: any) =>
            call.command.startsWith("desktop_set_") ||
            call.command === "desktop_quit",
        ),
      ),
    ).toBe(false);
    await context.close();
  });
}
