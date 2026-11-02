import { test, expect } from "@playwright/test";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";

test("full SQLite backup requires credential confirmation and publishes a complete snapshot", async ({
  page,
  request,
}) => {
  const workspace = await (
    await request.get("/api/v1/config/workspace")
  ).json();
  await page.goto("/console/settings.html");
  const panel = page.getByRole("region", { name: "数据库备份与恢复" });
  const button = panel.getByRole("button", { name: "下载 SQLite 一致备份" });
  await expect(button).toBeDisabled();
  await panel.getByRole("checkbox").check();
  await expect(button).toBeEnabled();
  if (workspace.storage_path) {
    const download = page.waitForEvent("download");
    await button.click();
    const artifact = await download;
    const file = await artifact.path();
    expect(file).not.toBeNull();
    const bytes = await readFile(file!);
    expect(bytes.subarray(0, 16).toString()).toBe("SQLite format 3\0");
    const digest = createHash("sha256").update(bytes).digest("hex");
    await expect(panel.getByRole("status")).toContainText(digest);
  } else {
    await button.click();
    await expect(panel.getByRole("status")).toContainText(
      "persistent data directory",
    );
  }
  await expect(panel).toContainText("先停止目标 Daemon");
  await expect(panel).toContainText("--offline");
  await page.setViewportSize({ width: 390, height: 844 });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
});

test("Linux diagnostics displays actual target topology and owned object counts without mutation", async ({
  page,
  request,
}) => {
  const snapshot = await (
    await request.get("/api/v1/linux/diagnostics")
  ).json();
  await page.goto("/console/linux.html");
  const panel = page.getByRole("region", { name: "只读拓扑与内核诊断" });
  await expect(panel).toContainText(
    snapshot.topology.kernel_release ?? "不支持",
  );
  await expect(panel).toContainText("已持有程序 IDs");
  await expect(panel).toContainText("不更改路由");
  await panel.getByRole("button", { name: "刷新并比较" }).click();
  await expect(panel.getByRole("button", { name: "刷新并比较" })).toBeEnabled();
  await page.setViewportSize({ width: 390, height: 844 });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
});
