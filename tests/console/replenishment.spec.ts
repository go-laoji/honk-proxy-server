import { test, expect } from "@playwright/test";
import { createServer, type ServerResponse } from "node:http";
import type { AddressInfo } from "node:net";

test("controller MPA shows reservations, pauses in-flight refill and resumes to target within shared budgets", async ({
  page,
  request,
}) => {
  const quantities: number[] = [];
  let first: ServerResponse | undefined;
  const body = (count: number, batch: number) =>
    JSON.stringify({
      data: {
        proxies: Array.from({ length: count }, (_, index) => ({
          ip: `198.51.100.${batch * 10 + index + 1}`,
          port: 8080,
          ttl: 300,
        })),
      },
    });
  const vendor = createServer((req, res) => {
    const quantity = Number(
      new URL(req.url!, "http://localhost").searchParams.get("num"),
    );
    quantities.push(quantity);
    if (quantities.length === 1) first = res;
    else {
      res.writeHead(200, { "content-type": "application/json" });
      res.end(body(quantity, quantities.length));
    }
  });
  await new Promise<void>((resolve) => vendor.listen(0, "127.0.0.1", resolve));
  const port = (vendor.address() as AddressInfo).port;
  try {
    await page.goto("/console/providers.html");
    await expect(
      page.getByRole("heading", { name: "供应商来源", exact: true }),
    ).toBeVisible();
    const form = page.getByRole("form", { name: "供应商配置" });
    await form.getByLabel("来源 ID", { exact: true }).fill("refill-vendor");
    await form.getByLabel("显示名称", { exact: true }).fill("预算控制测试");
    await form
      .getByLabel("提取 API 地址")
      .fill(`http://127.0.0.1:${port}/extract`);
    await form.getByLabel("启用供应商").check();
    await form.getByLabel("单次提取数量").fill("2");
    await form.getByText("请求参数与鉴权", { exact: true }).click();
    await form.getByLabel("数量参数名").fill("num");
    await form.getByText("租约与请求限制", { exact: true }).click();
    await form.getByLabel("最短提取间隔（秒）").fill("1");
    await form.getByText("自动补池", { exact: true }).click();
    await form.getByLabel("启用自动补池").check();
    await form.getByLabel("补池模式").selectOption("maintain");
    for (const [label, value] of Object.entries({
      最低节点数: "1",
      目标节点数: "3",
      最大节点数: "4",
      每批补充节点数: "2",
      "提前退出补池水位（秒）": "0",
      "预估使用窗口（秒）": "0",
      "空闲超时（秒）": "2",
      每日请求上限: "2",
      每日节点上限: "3",
      "首次退避间隔（秒）": "1",
      "最长退避间隔（秒）": "1",
      暂停前失败次数: "2",
    }))
      await form.getByLabel(label, { exact: true }).fill(value);
    await page
      .getByRole("button", { name: "保存并激活来源", exact: true })
      .click();
    await expect(page.getByRole("main").getByRole("status")).toContainText(
      "来源配置已激活",
    );
    await page.getByRole("link", { name: "预算控制测试", exact: true }).click();
    await expect(
      page.getByRole("region", { name: "补池控制器" }),
    ).toBeVisible();
    await expect.poll(() => quantities.length).toBe(1);
    await page
      .getByRole("button", { name: "暂停全部提取", exact: true })
      .click();
    await expect(page.getByRole("main").getByRole("status")).toContainText(
      "后续提取已暂停",
    );
    const reserved = await (
      await request.get("/api/v1/providers/refill-vendor/controller")
    ).json();
    expect([
      reserved.used_requests,
      reserved.reserved_nodes,
      reserved.inflight_nodes,
    ]).toEqual([1, 2, 2]);
    expect(reserved.paused_reason).toBe("manual");
    first!.writeHead(200, { "content-type": "application/json" });
    first!.end(body(2, 1));
    await expect(page.getByTestId("extraction-result")).toContainText("已入库");
    await expect(page.getByTestId("controller-decision")).toHaveText(
      "来源提取已暂停",
    );
    expect(quantities).toEqual([2]);
    await page
      .getByRole("button", { name: "确认结果并恢复提取", exact: true })
      .click();
    await expect(page.getByRole("main").getByRole("status")).toContainText(
      "预算和冷却保持不变",
    );
    await expect.poll(() => quantities.length).toBe(2);
    await expect(page.getByTestId("controller-decision")).toHaveText(
      "已达到目标库存",
    );
    await expect(page.getByText("2 / 2", { exact: true })).toBeVisible();
    await expect(page.getByText("3 / 3", { exact: true })).toBeVisible();
    expect(quantities).toEqual([2, 1]);
    await expect
      .poll(
        async () =>
          (
            await (
              await request.get("/api/v1/providers/refill-vendor/controller")
            ).json()
          ).next_allowed_at <=
          Date.now() / 1000,
      )
      .toBe(true);
    await page
      .getByRole("button", { name: "提取 2 个节点", exact: true })
      .click();
    await expect(page.getByRole("alert")).toContainText(
      "daily extraction budget exhausted",
    );
    expect(quantities).toEqual([2, 1]);
    const controller = await (
      await request.get("/api/v1/providers/refill-vendor/controller")
    ).json();
    expect([
      controller.used_requests,
      controller.reserved_nodes,
      controller.current_inventory,
    ]).toEqual([2, 3, 3]);
    await page.setViewportSize({ width: 390, height: 844 });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: "/tmp/hps-replenishment-console.png",
      fullPage: true,
      animations: "disabled",
    });
  } finally {
    const workspace = await (
      await request.get("/api/v1/config/workspace")
    ).json();
    if (workspace.desired_revision !== 1)
      await request.post("/api/v1/config/rollback", {
        data: { revision: 1, expected_revision: workspace.desired_revision },
      });
    vendor.closeAllConnections();
    await new Promise<void>((resolve) => vendor.close(() => resolve()));
  }
});
