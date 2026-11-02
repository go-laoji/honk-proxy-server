import { test, expect } from "@playwright/test";
import { createServer, type ServerResponse } from "node:http";
import type { AddressInfo } from "node:net";

test("provider MPA CRUD, conflicts, owned extraction recovery, redaction and lease publication use real HTTP", async ({
  page,
  request,
}) => {
  let calls = 0;
  const waiting: ServerResponse[] = [];
  const vendor = createServer((req, res) => {
    if (req.url?.startsWith("/extract")) {
      calls++;
      waiting.push(res);
    } else {
      res.writeHead(404);
      res.end();
    }
  });
  await new Promise<void>((resolve) => vendor.listen(0, "127.0.0.1", resolve));
  const port = (vendor.address() as AddressInfo).port;
  const peer = await page.context().newPage();
  const config = {
    id: "console-vendor",
    name: "控制台接口测试",
    enabled: true,
    kind: "json",
    endpoint: `http://127.0.0.1:${port}/extract`,
    protocol: "http",
    count: 2,
    min_interval_secs: 1,
    upstream_credentials: {
      username: "vendor",
      password: "console-vendor-private",
    },
  };
  try {
    await Promise.all([
      page.goto("/console/providers.html"),
      peer.goto("/console/providers.html"),
    ]);
    await expect(
      page.getByRole("heading", { name: "供应商来源", exact: true }),
    ).toBeVisible();
    await expect(
      peer.getByRole("heading", { name: "供应商来源", exact: true }),
    ).toBeVisible();
    await page
      .getByRole("textbox", { name: "供应商 JSON 配置" })
      .fill(JSON.stringify(config));
    await page
      .getByRole("button", { name: "保存并激活来源", exact: true })
      .click();
    await expect(page.getByRole("status")).toContainText("来源配置已激活");
    await peer
      .getByRole("textbox", { name: "供应商 JSON 配置" })
      .fill(JSON.stringify({ ...config, id: "stale-vendor" }));
    await peer
      .getByRole("button", { name: "保存并激活来源", exact: true })
      .click();
    await expect(peer.getByRole("alert")).toContainText(
      "configuration changed",
    );
    await page
      .getByRole("link", { name: "控制台接口测试", exact: true })
      .click();
    await expect(page).toHaveURL(/provider-detail\.html\?id=console-vendor$/);
    await expect(
      page.getByRole("textbox", { name: "供应商 JSON 配置" }),
    ).toHaveValue(/\[REDACTED\]/);
    await page
      .getByRole("button", { name: "提取 2 个节点", exact: true })
      .click();
    await expect(page.getByTestId("extraction-result")).toContainText("提取");
    await expect.poll(() => calls).toBe(1);
    const before = await (
      await request.get("/api/v1/providers/console-vendor")
    ).json();
    const taskId: number = before.tasks[0].id;
    const duplicate = await (
      await request.post("/api/v1/providers/console-vendor/extract", {
        data: { request_id: "second-click" },
      })
    ).json();
    expect(duplicate.id).toBe(taskId);
    await page.getByRole("link", { name: "代理入口", exact: true }).click();
    await page.goto("/console/provider-detail.html?id=console-vendor");
    await expect(page.getByTestId("extraction-result")).toContainText(
      "正在提取",
    );
    waiting[0].writeHead(200, { "content-type": "application/json" });
    waiting[0].end(
      JSON.stringify({
        data: { proxies: [{ ip: "192.0.2.1", port: 8080, ttl: 300 }] },
      }),
    );
    await expect(page.getByTestId("extraction-result")).toContainText("已入库");
    await expect(
      page.getByRole("cell", { name: "http://192.0.2.1:8080", exact: true }),
    ).toBeVisible();
    await expect(page.getByTestId("extraction-result")).toContainText("新增 1");
    expect(calls).toBe(1);
    expect(await page.locator("body").innerText()).not.toContain(
      "console-vendor-private",
    );
    const editor = page.getByRole("textbox", { name: "供应商 JSON 配置" });
    expect(await editor.inputValue()).not.toContain("console-vendor-private");
    await page
      .getByRole("button", { name: "保存并激活来源", exact: true })
      .click();
    await expect(page.getByRole("status")).toContainText("来源配置已激活");
    await expect(
      page.getByRole("cell", { name: "http://192.0.2.1:8080", exact: true }),
    ).toBeVisible();
    await page.setViewportSize({ width: 390, height: 844 });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: "/tmp/hps-provider-console.png",
      fullPage: true,
      animations: "disabled",
    });
    await page.goto("/console/providers.html");
    await page.getByRole("button", { name: "停用", exact: true }).click();
    await expect(page.getByRole("status")).toContainText("来源配置已激活");
    const disabled = await (
      await request.get("/api/v1/providers/console-vendor")
    ).json();
    expect(disabled.provider.configuration.enabled).toBe(false);
    expect(disabled.leases[0].state).toBe("source_disabled");
    expect(calls).toBe(1);
  } finally {
    const workspace = await (
      await request.get("/api/v1/config/workspace")
    ).json();
    if (workspace.desired_revision !== 1)
      await request.post("/api/v1/config/rollback", {
        data: { revision: 1, expected_revision: workspace.desired_revision },
      });
    await peer.close();
    vendor.closeAllConnections();
    await new Promise<void>((resolve) => vendor.close(() => resolve()));
  }
});
