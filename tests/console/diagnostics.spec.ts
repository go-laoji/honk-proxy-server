import { test, expect } from "@playwright/test";
import { readFile } from "node:fs/promises";
test("diagnostic task survives navigation and exports only bounded metadata through shared pages", async ({
  page,
  request,
}) => {
  await page.goto("/console/events.html");
  const panel = page.getByRole("region", { name: "脱敏诊断包" });
  await panel.getByRole("button", { name: "创建诊断包" }).click();
  await expect(page).toHaveURL(/diagnostic=\d+&epoch=/);
  await expect(
    panel.getByRole("button", { name: "导出诊断 JSON" }),
  ).toBeVisible();
  const url = new URL(page.url()),
    id = url.searchParams.get("diagnostic")!,
    epoch = url.searchParams.get("epoch")!;
  await page.goto(`/console/tasks.html?detail=${id}&epoch=${epoch}`);
  await expect(page.getByRole("group", { name: "任务控制" })).toBeVisible();
  await expect(page.getByText("预览并导出诊断包")).toBeVisible();
  await expect(page.getByRole("button", { name: "取消只读诊断" })).toHaveCount(
    0,
  );
  await page.getByText("预览并导出诊断包").click();
  await expect(panel.getByText("导出内容预览")).toBeVisible();
  const downloading = page.waitForEvent("download");
  await panel.getByRole("button", { name: "导出诊断 JSON" }).click();
  const download = await downloading;
  expect(download.suggestedFilename()).toBe(`hps-diagnostic-${id}.json`);
  const data = JSON.parse(await readFile((await download.path())!, "utf8"));
  expect(data.format_version).toBe(1);
  expect(data.consistent_database_backup).toBe(false);
  expect(data.sample_count).toBeLessThanOrEqual(721);
  expect(JSON.stringify(data)).not.toMatch(
    /127\.0\.0\.1|\/Users\/|"credentials"\s*:|"api_key"\s*:/,
  );
  const controls = await (
    await request.get(`/api/v1/tasks/${id}/controls?expected_epoch=${epoch}`)
  ).json();
  expect(controls.can_cancel).toBe(false);
  expect(controls.can_retry).toBe(false);
  await page.setViewportSize({ width: 390, height: 844 });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
});
