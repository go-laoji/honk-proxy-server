import { test, expect } from "@playwright/test";

test("all MPA documents serve the shared brand icons through the daemon", async ({
  request,
}) => {
  const manifest = await (
    await request.get("/console/web-manifest.json")
  ).json();
  expect(manifest.pages).toHaveLength(18);
  for (const file of manifest.pages) {
    const response = await request.get(`/console/${file}`);
    expect(response.ok(), file).toBe(true);
    const html = await response.text();
    expect(html, file).toContain('href="./brand/favicon.svg"');
    expect(html, file).toContain('href="./brand/favicon.ico"');
    expect(html, file).toContain('href="./brand/apple-touch-icon.png"');
    expect(html, file).toContain('href="./brand/site.webmanifest"');
  }
  for (const [file, mime] of [
    ["favicon.svg", "image/svg+xml"],
    ["favicon.ico", "image/vnd.microsoft.icon"],
    ["apple-touch-icon.png", "image/png"],
    ["icon-192.png", "image/png"],
    ["icon-512.png", "image/png"],
    ["site.webmanifest", "application/manifest+json"],
  ]) {
    const response = await request.get(`/console/brand/${file}`);
    expect(response.ok(), file).toBe(true);
    expect(response.headers()["content-type"], file).toContain(mime);
    expect(response.headers()["x-hps-web-build-id"], file).toBe(
      manifest.web_build_id,
    );
    expect(manifest.assets[`brand/${file}`]).toMatch(/^[a-f0-9]{64}$/);
  }
});

test("brand artwork follows the selected theme and survives MPA navigation", async ({
  page,
}) => {
  await page.goto("/console/overview.html");
  const light = page.locator('img[data-brand="light"]');
  const dark = page.locator('img[data-brand="dark"]');
  await expect(light).toBeVisible();
  await expect(dark).toBeHidden();
  expect(
    await light.evaluate((image: HTMLImageElement) => image.naturalWidth),
  ).toBe(320);
  await page.getByRole("button", { name: "切换深色模式" }).click();
  await expect(dark).toBeVisible();
  await expect(light).toBeHidden();
  expect(
    await dark.evaluate((image: HTMLImageElement) => image.naturalWidth),
  ).toBe(320);
  await page.getByRole("link", { name: "系统设置", exact: true }).click();
  await expect(dark).toBeVisible();
  await expect(page.getByRole("link", { name: /Honk Proxy/ })).toBeVisible();
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.getByRole("link", { name: /Honk Proxy/ }).hover();
  expect(
    await page
      .locator(".brand-logo")
      .evaluate((node) => getComputedStyle(node).transform),
  ).toBe("none");
});
