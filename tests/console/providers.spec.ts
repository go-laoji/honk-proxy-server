import { test, expect } from "@playwright/test";
import { createServer, type ServerResponse } from "node:http";
import type { AddressInfo } from "node:net";

test("provider MPA CRUD, conflicts, owned extraction recovery, redaction and lease publication use real HTTP", async ({
  page,
  request,
}) => {
  let calls = 0;
  let forwardedHeader: string | string[] | undefined;
  let forwardedParameter: string | null = null;
  const waiting: ServerResponse[] = [];
  const vendor = createServer((req, res) => {
    if (req.url?.startsWith("/extract")) {
      calls++;
      forwardedHeader = req.headers["x-console-fixture"];
      forwardedParameter = new URL(
        req.url,
        "http://localhost",
      ).searchParams.get(" exact ");
      waiting.push(res);
    } else {
      res.writeHead(404);
      res.end();
    }
  });
  await new Promise<void>((resolve) => vendor.listen(0, "127.0.0.1", resolve));
  const port = (vendor.address() as AddressInfo).port;
  const peer = await page.context().newPage();
  async function fillProvider(target: typeof page, id: string) {
    const form = target.getByRole("form", { name: "供应商配置" });
    await form.getByLabel("来源 ID", { exact: true }).fill(id);
    await form.getByLabel("显示名称", { exact: true }).fill("控制台接口测试");
    await form
      .getByLabel("提取 API 地址")
      .fill(`http://127.0.0.1:${port}/extract`);
    await form.getByLabel("单次提取数量").fill("2");
    await form.getByLabel("启用供应商").check();
    await form.getByText("租约与请求限制", { exact: true }).click();
    await form.getByLabel("最短提取间隔（秒）").fill("1");
    await form.getByText("请求参数与鉴权", { exact: true }).click();
    await form.getByLabel("配置节点代理凭据").check();
    await form.getByLabel("节点代理用户名").fill("vendor");
    await form.getByLabel("节点代理密码").fill("console-vendor-private");
  }
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
    const newProvider = page.getByRole("form", { name: "供应商配置" });
    await newProvider
      .getByLabel("来源 ID", { exact: true })
      .fill("console-vendor");
    await newProvider
      .getByLabel("提取 API 地址")
      .fill(`http://127.0.0.1:${port}/extract`);
    const requestSettings = newProvider.getByText("请求参数与鉴权", {
      exact: true,
    });
    await requestSettings.click();
    const auth = newProvider.getByRole("combobox", {
      name: "鉴权方式",
      exact: true,
    });
    await auth.selectOption("token");
    await requestSettings.click();
    await newProvider.getByRole("button", { name: "保存并激活来源" }).click();
    await expect(
      newProvider.getByLabel("Secret ID", { exact: true }),
    ).toBeFocused();
    await auth.selectOption("none");
    await requestSettings.click();
    await fillProvider(page, "console-vendor");
    const params = page.getByRole("group", { name: "请求参数", exact: true });
    await params.getByRole("button", { name: "添加请求参数" }).click();
    await params.getByLabel("参数名", { exact: true }).fill(" exact ");
    await params.getByLabel("参数值", { exact: true }).fill("sent");
    const headers = page.getByRole("group", { name: "请求头", exact: true });
    await headers.getByRole("button", { name: "添加请求头" }).click();
    await headers.getByLabel("请求头名称").fill("X-Console-Fixture");
    await headers.getByLabel("请求头值").fill("fixture-value");
    await headers.getByRole("button", { name: "添加请求头" }).click();
    await headers.getByLabel("请求头名称").last().fill("X-Console-Fixture");
    await expect(headers.getByText("名称已存在", { exact: true })).toHaveCount(
      2,
    );
    await headers.getByRole("button", { name: "移除请求头 2" }).click();

    await page
      .getByRole("button", { name: "保存并激活来源", exact: true })
      .click();
    await expect(page.getByRole("main").getByRole("status")).toContainText(
      "来源配置已激活",
    );
    await fillProvider(peer, "stale-vendor");
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
    const form = page.getByRole("form", { name: "供应商配置" });
    await form.getByText("请求参数与鉴权", { exact: true }).click();
    const password = form.getByLabel("节点代理密码");
    await expect(password).toHaveValue("[REDACTED]");
    await expect(password).toHaveAttribute("type", "password");
    await page
      .getByRole("button", { name: "提取 2 个节点", exact: true })
      .click();
    await expect(page.getByTestId("extraction-result")).toContainText("提取");
    await expect.poll(() => calls).toBe(1);
    expect(forwardedHeader).toBe("fixture-value");
    expect(forwardedParameter).toBe("sent");
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
    await expect(password).toHaveValue("[REDACTED]");
    await page
      .getByRole("button", { name: "保存并激活来源", exact: true })
      .click();
    await expect(page.getByRole("main").getByRole("status")).toContainText(
      "来源配置已激活",
    );
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
    await expect(page.getByRole("main").getByRole("status")).toContainText(
      "来源配置已激活",
    );
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
