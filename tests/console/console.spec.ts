import { expect, test } from "@playwright/test";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";

const pages: { id: string; file: string; title: string }[] = JSON.parse(
  readFileSync("web/console/pages.manifest.json", "utf8"),
);

test("all MPA documents are distinct, directly accessible and never use a login/SPA fallback", async ({
  request,
}) => {
  expect(pages).toHaveLength(18);
  for (const page of pages) {
    const response = await request.get(`/console/${page.file}?direct=1`);
    expect(response.status()).toBe(200);
    expect(response.headers()["content-type"]).toContain("text/html");
    expect(response.headers()["cache-control"]).toBe("no-cache");
    expect(await response.text()).toContain(`data-page="${page.id}"`);
  }
  expect((await request.get("/console/missing.html")).status()).toBe(404);
  expect((await request.get("/console/login.html")).status()).toBe(404);
  expect((await request.get("/api/v1/session")).status()).toBe(404);
  expect((await request.get("/readyz")).status()).toBe(503);
});

test("embedded server assets match the shared release manifest byte for byte", async ({
  request,
}) => {
  const manifest = await (
    await request.get("/console/web-manifest.json")
  ).json();
  const version = await (await request.get("/api/v1/version")).json();
  expect(version.web_build_id).toBe(manifest.web_build_id);
  for (const [path, digest] of Object.entries(manifest.assets)) {
    const response = await request.get(`/console/${path}`);
    expect(response.status(), path).toBe(200);
    expect(
      createHash("sha256")
        .update(await response.body())
        .digest("hex"),
      path,
    ).toBe(digest);
    expect(response.headers()["x-hps-web-build-id"]).toBe(
      manifest.web_build_id,
    );
  }
});

test("overview uses real API state and navigation creates a new document", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto("/console/overview.html");
  await expect(
    page.getByRole("heading", { name: "运行概览", exact: true }),
  ).toBeVisible();
  await expect(page.getByText("代理服务尚未就绪")).toBeVisible();
  await expect(
    page.getByText("honk-proxy-server", { exact: true }).first(),
  ).toBeVisible();
  await page.screenshot({
    path: "/tmp/hps-runtime-overview.png",
    fullPage: true,
  });
  await page.evaluate(() => {
    Object.assign(window, { navigationProbe: true });
  });
  await page.getByRole("link", { name: "上游供应商", exact: true }).click();
  await expect(page).toHaveURL(/providers\.html$/);
  expect(await page.evaluate(() => "navigationProbe" in window)).toBe(false);
  await expect(
    page.getByRole("heading", { name: "上游供应商当前不可用" }),
  ).toBeVisible();
  await page.reload();
  await expect(
    page.getByRole("heading", { name: "上游供应商", exact: true }),
  ).toBeVisible();
  await page.goBack();
  await expect(
    page.getByRole("heading", { name: "运行概览", exact: true }),
  ).toBeVisible();
  expect(errors).toEqual([]);
});

test("mobile navigation, theme persistence and no-account settings work", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/console/overview.html");
  await expect(page.getByText("代理服务尚未就绪")).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
  await page.getByRole("button", { name: "切换深色模式" }).click();
  await expect(page.locator("html")).toHaveClass("dark");
  await page.getByRole("button", { name: "打开导航" }).click();
  await page.getByRole("link", { name: "系统设置", exact: true }).click();
  await expect(page.getByText("无需登录，直接访问全部功能")).toBeVisible();
  await expect(page.locator("html")).toHaveClass("dark");
  expect((await page.context().cookies()).length).toBe(0);
});
