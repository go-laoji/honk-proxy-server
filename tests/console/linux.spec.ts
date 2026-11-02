import { test, expect } from "@playwright/test";
test("Linux MPA reads actual daemon prerequisites without claiming capture", async ({
  page,
  request,
}) => {
  const report = await (await request.get("/api/v1/linux/preflight")).json();
  await page.goto("/console/linux.html");
  const region = page.getByRole("region", { name: "Linux 环境预检" });
  await expect(region).toContainText(report.platform);
  for (const check of report.checks)
    await expect(region.getByText(check.id, { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "刷新预检", exact: true }).click();
  await expect(region).toContainText("不加载程序");
  await expect(
    page.getByText(
      "hps --output json linux self-test --object /opt/hps/hps-ebpf",
      { exact: true },
    ),
  ).toBeVisible();
  await page.setViewportSize({ width: 390, height: 844 });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
});
