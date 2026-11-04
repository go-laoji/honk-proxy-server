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
    const config = {
      id: "refill-vendor",
      name: "预算控制测试",
      enabled: true,
      kind: "json",
      endpoint: `http://127.0.0.1:${port}/extract`,
      count: 2,
      count_parameter: "num",
      min_interval_secs: 1,
      refill: {
        enabled: true,
        mode: "maintain",
        min_nodes: 1,
        target_nodes: 3,
        max_nodes: 4,
        batch_size: 2,
        retire_before_secs: 0,
        forecast_secs: 0,
        idle_timeout_secs: 2,
        daily_request_budget: 2,
        daily_node_budget: 3,
        backoff_initial_secs: 1,
        backoff_max_secs: 1,
        failure_limit: 2,
      },
    };
    await page
      .getByRole("textbox", { name: "供应商 JSON 配置" })
      .fill(JSON.stringify(config));
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
