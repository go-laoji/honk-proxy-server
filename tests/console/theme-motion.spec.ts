import { expect, test, type Page } from "@playwright/test";

async function contrast(page: Page, text: string, background: string) {
  return page.evaluate(
    ({ text, background }) => {
      const canvas = document.createElement("canvas");
      canvas.width = canvas.height = 1;
      const context = canvas.getContext("2d")!;
      const luminance = (color: string) => {
        context.clearRect(0, 0, 1, 1);
        context.fillStyle = color;
        context.fillRect(0, 0, 1, 1);
        const channels = [...context.getImageData(0, 0, 1, 1).data]
          .slice(0, 3)
          .map((n) => {
            const value = n / 255;
            return value <= 0.04045
              ? value / 12.92
              : ((value + 0.055) / 1.055) ** 2.4;
          });
        return (
          channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722
        );
      };
      const foreground = luminance(
        getComputedStyle(document.querySelector(text)!).color,
      );
      const surface = luminance(
        getComputedStyle(document.querySelector(background)!).backgroundColor,
      );
      return (
        (Math.max(foreground, surface) + 0.05) /
        (Math.min(foreground, surface) + 0.05)
      );
    },
    { text, background },
  );
}

test("both themes keep navigation, primary actions and secondary text readable across MPA navigation", async ({
  page,
}, testInfo) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto("/console/index.html");
  await expect(page.getByText("当前服务可达", { exact: true })).toBeVisible();
  for (const mode of ["light", "dark"]) {
    if (mode === "dark")
      await page.getByRole("button", { name: "切换深色模式" }).click();
    for (const [text, background] of [
      [".nav-link:not([aria-current])", ".sidebar"],
      [".nav-link[aria-current]", ".nav-link[aria-current]"],
      [".nav-group-label", ".sidebar"],
      [".btn-primary", ".btn-primary"],
      [".panel p.muted", ".panel"],
    ])
      expect(
        await contrast(page, text, background),
        `${mode} ${text}`,
      ).toBeGreaterThanOrEqual(4.5);
    await page.getByRole("link", { name: "运行概览", exact: true }).click();
    await expect(page.locator(".stat-card").first()).toBeVisible();
    await expect(page.getByRole("figure").first()).toBeVisible();
    await page.screenshot({
      path: testInfo.outputPath(`overview-${mode}.png`),
      fullPage: true,
    });
    if (mode === "light")
      await page.getByRole("link", { name: "实例连接", exact: true }).click();
  }
  await page.setViewportSize({ width: 390, height: 844 });
  await page.getByRole("button", { name: "打开导航" }).click();
  await page.screenshot({
    path: testInfo.outputPath("mobile-dark-navigation.png"),
  });
  await page.getByRole("link", { name: "系统设置", exact: true }).click();
  await expect(page.locator("html")).toHaveClass("dark");
  await expect(page.getByText("无需登录，直接访问全部功能")).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: testInfo.outputPath("mobile-dark-settings.png"),
    fullPage: true,
  });
});

test("actual refresh motion stops with reduced motion while loading stays understandable", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await page.goto("/console/overview.html");
  await expect(page.locator(".stat-card").first()).toBeVisible();
  let resume!: () => void;
  const gate = new Promise<void>((resolve) => {
    resume = resolve;
  });
  await page.route("**/api/v1/status", async (route) => {
    await gate;
    await route.continue();
  });
  const refresh = page.getByRole("button", { name: "刷新状态", exact: true });
  try {
    await refresh.click();
    await expect(refresh).toBeDisabled();
    const icon = refresh.locator(".action-icon");
    await expect(icon).toHaveCSS("animation-name", "icon-spin");
    await page.emulateMedia({ reducedMotion: "reduce" });
    await expect(icon).toHaveCSS("animation-name", "none");
    await expect(refresh).toContainText("刷新状态");
    const link = page.getByRole("link", { name: "上游供应商", exact: true });
    await link.hover();
    await expect(link.locator(".nav-icon")).toHaveCSS("transform", "none");
  } finally {
    resume();
  }
  await expect(refresh).toBeEnabled();
  await page.unroute("**/api/v1/status");
});
