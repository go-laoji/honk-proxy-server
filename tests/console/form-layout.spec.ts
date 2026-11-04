import { expect, test } from "@playwright/test";

test("node filters align their labels, fields and actions on one row", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/console/nodes.html");
  const filters = page.getByRole("form", { name: "节点筛选" });
  const controls = [
    filters.getByRole("textbox", { name: "名称或端点" }),
    filters.getByRole("combobox", { name: "来源", exact: true }),
    filters.getByRole("combobox", { name: "状态", exact: true }),
    filters.getByRole("button", { name: "应用筛选" }),
    filters.getByRole("button", { name: "重置" }),
  ];
  await expect(controls[0]).toBeVisible();

  const boxes = await Promise.all(
    controls.map((control) => control.boundingBox()),
  );
  for (const box of boxes) {
    expect(box).not.toBeNull();
    expect(box!.height).toBeCloseTo(boxes[0]!.height, 0);
    expect(box!.y).toBeCloseTo(boxes[0]!.y, 0);
  }

  const labelTops = await filters
    .locator("label")
    .filter({ visible: true })
    .evaluateAll((labels) =>
      labels.map((label) => label.getBoundingClientRect().top),
    );
  expect(labelTops).toHaveLength(3);
  expect(Math.max(...labelTops) - Math.min(...labelTops)).toBeLessThan(1);

  await page.setViewportSize({ width: 390, height: 844 });
  await controls[2].selectOption("expired");
  await controls[3].click();
  await expect(page).toHaveURL(/state=expired/);
  await expect(controls[2]).toHaveValue("expired");

  await page.getByRole("button", { name: "手动添加", exact: true }).click();
  const form = page.getByRole("form", { name: "单个代理节点" });
  const date = form.getByLabel("到期时间（可选，本地时间）");
  await expect(date).toBeVisible();
  expect((await date.boundingBox())!.height).toBeCloseTo(boxes[0]!.height, 0);
  expect(
    await page.evaluate(() => document.documentElement.scrollWidth),
  ).toBeLessThanOrEqual(390);
});
