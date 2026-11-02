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
    const policyEditor = page.getByRole("textbox", {
      name: "规则策略 JSON",
      exact: true,
    });
    const policy = JSON.parse(await policyEditor.inputValue());
    policy.default_action = { kind: "direct" };
    await policyEditor.fill(JSON.stringify(policy));
    await page.getByRole("button", { name: "保存为候选", exact: true }).click();
    await expect(page.getByRole("status")).toContainText("需要显式激活");
    expect((await throughProxy(proxy, target)).status).toBe(503);
    await page
      .getByRole("textbox", { name: "连接上下文 JSON", exact: true })
      .fill(
        JSON.stringify({
          destination_host: "127.0.0.1",
          destination_port: port,
          transport: "tcp",
          source_ip: "127.0.0.1",
          inbound: "browser-http",
          inbound_protocol: "http",
        }),
      );
    const result = page.getByLabel("规则模拟结果", { exact: true });
    await page
      .getByRole("button", { name: "执行规则模拟", exact: true })
      .click();
    await expect(result).toContainText("明确直连");
    await page
      .getByRole("button", { name: "激活查看的候选", exact: true })
      .click();
    await expect(page.getByRole("status")).toContainText("激活结果 applied");
    expect(await throughProxy(proxy, target)).toEqual({
      status: 200,
      body: "rule-origin",
    });
    async function add(id: string, kind: string) {
      await page.getByRole("button", { name: "新增规则", exact: true }).click();
      await page
        .getByRole("textbox", { name: "规则 JSON", exact: true })
        .fill(
          JSON.stringify({
            id,
            enabled: true,
            conditions: { destination_ports: [{ start: port, end: port }] },
            action: { kind },
          }),
        );
      await page
        .getByRole("button", { name: "保存为候选", exact: true })
        .click();
      await expect(page.getByRole("status")).toContainText("需要显式激活");
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
    await expect(page.getByRole("status")).toContainText("激活结果 applied");
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
    await expect(page.getByRole("status")).toContainText("激活结果 applied");
    expect((await throughProxy(proxy, target)).status).toBe(200);
    await page
      .getByRole("button", { name: "停用 allow-local", exact: true })
      .click();
    await expect(page.getByRole("status")).toContainText("需要显式激活");
    expect((await throughProxy(proxy, target)).status).toBe(200);
    await page
      .getByRole("button", { name: "激活查看的候选", exact: true })
      .click();
    await expect(page.getByRole("status")).toContainText("激活结果 applied");
    expect((await throughProxy(proxy, target)).status).toBe(403);
    await page
      .getByRole("button", { name: "删除 deny-local", exact: true })
      .click();
    await expect(page.getByRole("status")).toContainText("需要显式激活");
    await page
      .getByRole("button", { name: "激活查看的候选", exact: true })
      .click();
    await expect(page.getByRole("status")).toContainText("激活结果 applied");
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
    await page
      .getByRole("textbox", { name: "规则 JSON", exact: true })
      .fill(
        JSON.stringify({
          id: "process-future",
          enabled: false,
          conditions: { process: { command: "browser" } },
          action: { kind: "reject" },
        }),
      );
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
