import { test, expect } from "@playwright/test";
import { createServer, request as httpRequest } from "node:http";
import type { AddressInfo } from "node:net";

function throughProxy(
  proxy: string,
  target: string,
): Promise<{ status: number; body: string }> {
  const address = new URL(`http://${proxy}`);
  return new Promise((resolve, reject) => {
    const req = httpRequest(
      {
        hostname: address.hostname,
        port: address.port,
        path: target,
        headers: { Host: new URL(target).host, Connection: "close" },
      },
      (res) => {
        let body = "";
        res.on("data", (chunk) => {
          body += String(chunk);
        });
        res.on("end", () => resolve({ status: res.statusCode ?? 0, body }));
      },
    );
    req.setTimeout(5000, () => req.destroy(new Error("proxy fixture timeout")));
    req.on("error", reject);
    req.end();
  });
}
test("rules MPA separates candidates and runtime, orders by keyboard/buttons and applies to real HTTP traffic", async ({
  page,
  request,
}) => {
  const origin = createServer((_req, res) => {
    res.writeHead(200, { Connection: "close" });
    res.end("rule-origin");
  });
  await new Promise<void>((resolve) => origin.listen(0, "127.0.0.1", resolve));
  try {
    expect(
      (await request.post("/api/v1/inbounds/browser-http/start")).ok(),
    ).toBe(true);
    const inbounds = await (await request.get("/api/v1/inbounds")).json();
    const proxy: string = inbounds.inbounds.find(
      (i: { id: string }) => i.id === "browser-http",
    ).listen;
    const port = (origin.address() as AddressInfo).port;
    const target = `http://127.0.0.1:${port}/`;
    expect((await throughProxy(proxy, target)).status).toBe(503);
    await page.goto("/console/rules.html");
    await expect(
      page.getByRole("heading", { name: "规则与运行版本", exact: true }),
    ).toBeVisible();
    await expect(
      page.getByText("本机进程来源不可用", { exact: true }),
    ).toBeVisible();
    await page
      .getByRole("button", { name: "编辑规则策略", exact: true })
      .click();
    const editor = page.getByRole("form", { name: "规则配置" });
    const action = editor.getByRole("combobox", {
      name: "流量动作",
      exact: true,
    });
    await action.selectOption("group");
    await editor.getByLabel("目标节点或分组 ID").fill("old-draft-target");
    await action.selectOption("direct");
    await page.getByRole("button", { name: "保存为候选", exact: true }).click();
    await expect(page.getByRole("main").getByRole("status")).toContainText(
      "需要显式激活",
    );
    expect((await throughProxy(proxy, target)).status).toBe(503);
    const context = page.getByRole("form", { name: "规则模拟条件" });
    await context.getByLabel("目标域名或 IP").fill("127.0.0.1");
    await context.getByLabel("目标端口").fill(String(port));
    await context.getByLabel("入口 ID").fill("browser-http");
    await context.getByText("来源、DNS 观测与进程", { exact: true }).click();
    await context.getByLabel("来源 IP", { exact: true }).fill("127.0.0.1");
    await context
      .getByRole("combobox", { name: "入口协议", exact: true })
      .selectOption("http");
    const result = page.getByLabel("规则模拟结果", { exact: true });
    await page
      .getByRole("button", { name: "执行规则模拟", exact: true })
      .click();
    await expect(result).toContainText("明确直连");
    await page
      .getByRole("button", { name: "激活查看的候选", exact: true })
      .click();
    await expect(page.getByRole("main").getByRole("status")).toContainText(
      "激活结果 applied",
    );
    expect(await throughProxy(proxy, target)).toEqual({
      status: 200,
      body: "rule-origin",
    });
    async function add(id: string, kind: string) {
      await page.getByRole("button", { name: "新增规则", exact: true }).click();
      await editor.getByLabel("规则 ID").fill(id);
      await editor.getByLabel("启用此规则").check();
      await editor
        .getByRole("combobox", { name: "流量动作", exact: true })
        .selectOption(kind);
      await editor.getByRole("button", { name: "添加目标端口范围" }).click();
      await editor.getByLabel("起始端口").fill(String(port));
      await editor.getByLabel("结束端口").fill(String(port));
      await page
        .getByRole("button", { name: "保存为候选", exact: true })
        .click();
      await expect(page.getByRole("main").getByRole("status")).toContainText(
        "需要显式激活",
      );
    }
    await add("deny-local", "reject");
    expect((await throughProxy(proxy, target)).status).toBe(200);
    await page
      .getByRole("button", { name: "执行规则模拟", exact: true })
      .click();
    await expect(result).toContainText("命中 deny-local");
    await page
      .getByRole("button", { name: "激活查看的候选", exact: true })
      .click();
    await expect(page.getByRole("main").getByRole("status")).toContainText(
      "激活结果 applied",
    );
    expect((await throughProxy(proxy, target)).status).toBe(403);
    await add("allow-local", "direct");
    const allow = page
      .getByRole("article")
      .filter({ has: page.getByRole("heading", { name: /allow-local/ }) });
    await allow.focus();
    await allow.press("Alt+ArrowUp");
    await expect(
      page.getByRole("heading", { name: "1. allow-local", exact: true }),
    ).toBeVisible();
    await page
      .getByRole("button", { name: "下移规则 allow-local", exact: true })
      .click();
    await expect(
      page.getByRole("heading", { name: "1. deny-local", exact: true }),
    ).toBeVisible();
    await page
      .getByRole("button", { name: "上移规则 allow-local", exact: true })
      .click();
    await expect(
      page.getByRole("heading", { name: "1. allow-local", exact: true }),
    ).toBeVisible();
    expect((await throughProxy(proxy, target)).status).toBe(403);
    await page
      .getByRole("button", { name: "执行规则模拟", exact: true })
      .click();
    await expect(result).toContainText("命中 allow-local");
    await page
      .getByRole("button", { name: "激活查看的候选", exact: true })
      .click();
    await expect(page.getByRole("main").getByRole("status")).toContainText(
      "激活结果 applied",
    );
    expect((await throughProxy(proxy, target)).status).toBe(200);
    await page
      .getByRole("button", { name: "停用 allow-local", exact: true })
      .click();
    await expect(page.getByRole("main").getByRole("status")).toContainText(
      "需要显式激活",
    );
    expect((await throughProxy(proxy, target)).status).toBe(200);
    await page
      .getByRole("button", { name: "激活查看的候选", exact: true })
      .click();
    await expect(page.getByRole("main").getByRole("status")).toContainText(
      "激活结果 applied",
    );
    expect((await throughProxy(proxy, target)).status).toBe(403);
    await page
      .getByRole("button", { name: "删除 deny-local", exact: true })
      .click();
    await expect(page.getByRole("main").getByRole("status")).toContainText(
      "需要显式激活",
    );
    await page
      .getByRole("button", { name: "激活查看的候选", exact: true })
      .click();
    await expect(page.getByRole("main").getByRole("status")).toContainText(
      "激活结果 applied",
    );
    expect((await throughProxy(proxy, target)).status).toBe(200);

    await page
      .getByRole("button", { name: "编辑规则策略", exact: true })
      .click();
    const snapshot = await (await request.get("/api/v1/rules")).json();
    snapshot.configuration.use_dns_observations = true;
    const concurrent = await (
      await request.post("/api/v1/rules/save", {
        data: {
          configuration: snapshot.configuration,
          expected_revision: snapshot.running_revision,
        },
      })
    ).json();
    expect(
      (
        await request.post("/api/v1/rules/apply", {
          data: {
            revision: concurrent.id,
            expected_revision: snapshot.running_revision,
          },
        })
      ).ok(),
    ).toBe(true);
    await page.getByRole("button", { name: "保存为候选", exact: true }).click();
    await expect(page.getByRole("alert")).toContainText(
      "configuration changed",
    );
    await page.getByRole("button", { name: "取消编辑", exact: true }).click();
    await page.getByRole("button", { name: "刷新规则", exact: true }).click();
    await expect(
      page.getByRole("button", { name: "基于当前版本另存候选", exact: true }),
    ).toBeVisible();
    await page
      .getByRole("combobox", { name: "查看版本", exact: true })
      .selectOption(String(concurrent.id));
    await page.getByRole("button", { name: "新增规则", exact: true }).click();
    await editor.getByLabel("规则 ID").fill("process-future");
    await editor.getByText("入口、来源与本机进程", { exact: true }).click();
    await editor.getByLabel("配置进程条件").check();
    await editor.getByLabel("进程名称").fill("browser");
    await page.getByRole("button", { name: "保存为候选", exact: true }).click();
    await expect(
      page.getByRole("button", { name: "启用 process-future", exact: true }),
    ).toBeDisabled();
    await page.setViewportSize({ width: 390, height: 844 });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: "/tmp/hps-t11-rules-mobile.png",
      fullPage: true,
      animations: "disabled",
    });
  } finally {
    const workspace = await (
      await request.get("/api/v1/config/workspace")
    ).json();
    await request.post("/api/v1/config/rollback", {
      data: { revision: 1, expected_revision: workspace.desired_revision },
    });
    await new Promise<void>((resolve, reject) =>
      origin.close((error) => (error ? reject(error) : resolve())),
    );
  }
});
