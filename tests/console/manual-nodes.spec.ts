import { test, expect, type APIRequestContext } from "@playwright/test";
import http from "node:http";
import type { AddressInfo } from "node:net";

async function reset(request: APIRequestContext) {
  await request.post("/api/v1/inbounds/browser-http/stop");
  const current = await (await request.get("/api/v1/config/workspace")).json();
  expect(
    (
      await request.post("/api/v1/config/rollback", {
        data: { revision: 1, expected_revision: current.desired_revision },
      })
    ).ok(),
  ).toBe(true);
}

async function traffic(port: number, target: string): Promise<string> {
  return new Promise((resolve, reject) => {
    const req = http.get(
      { host: "127.0.0.1", port, path: target, agent: false },
      (res) => {
        const chunks: Buffer[] = [];
        res.on("data", (chunk: Buffer) => chunks.push(chunk));
        res.on("end", () =>
          res.statusCode === 200
            ? resolve(Buffer.concat(chunks).toString())
            : reject(new Error(`HTTP ${res.statusCode}`)),
        );
      },
    );
    req.on("error", reject);
    req.setTimeout(5000, () =>
      req.destroy(new Error("manual node traffic timeout")),
    );
  });
}

test("manual node form persists authenticated HTTP node, passes health and carries real traffic", async ({
  page,
  request,
}) => {
  let authenticated = 0;
  let originRequests = 0;
  let leakedProxyAuth = false;
  const username = "manual-user";
  const password = "pass@:# +中文";
  const origin = http.createServer((req, res) => {
    originRequests++;
    leakedProxyAuth ||= !!req.headers["proxy-authorization"];
    res.end("203.0.113.41\n");
  });
  await new Promise<void>((resolve) => origin.listen(0, "127.0.0.1", resolve));
  const originPort = (origin.address() as AddressInfo).port;
  const upstream = http.createServer((req, res) => {
    if (
      req.headers["proxy-authorization"] !==
      `Basic ${Buffer.from(`${username}:${password}`).toString("base64")}`
    ) {
      res.writeHead(407);
      res.end();
      return;
    }
    authenticated++;
    const target = new URL(req.url!);
    if (target.hostname !== "127.0.0.1" || Number(target.port) !== originPort) {
      res.writeHead(403);
      res.end();
      return;
    }
    const forwarded = http.request(
      {
        hostname: target.hostname,
        port: target.port,
        path: target.pathname,
        method: req.method,
      },
      (incoming) => {
        res.writeHead(incoming.statusCode!, incoming.headers);
        incoming.pipe(res);
      },
    );
    forwarded.on("error", () => {
      res.writeHead(502);
      res.end();
    });
    req.pipe(forwarded);
  });
  await new Promise<void>((resolve) =>
    upstream.listen(0, "127.0.0.1", resolve),
  );
  const port = (upstream.address() as AddressInfo).port;
  try {
    await reset(request);
    const workspace = await (
      await request.get("/api/v1/config/workspace")
    ).json();
    const health = await (await request.get("/api/v1/health")).json();
    expect(
      (
        await request.post("/api/v1/health/save", {
          data: {
            expected_revision: workspace.desired_revision,
            configuration: {
              ...health.configuration,
              enabled: true,
              daily_probe_budget: 10,
              profiles: [
                {
                  transport: "tcp",
                  family: "ipv4",
                  enabled: true,
                  url: `http://127.0.0.1:${originPort}/health`,
                  expected_status: 200,
                  capture_exit_ip: true,
                },
              ],
            },
          },
        })
      ).ok(),
    ).toBe(true);
    await page.goto("/console/nodes.html?protocol=socks5");
    await page.getByRole("button", { name: "手动添加", exact: true }).click();
    const form = page.getByRole("form", { name: "单个代理节点" });
    await expect(
      page.getByLabel("代理链接（可选，用于快速填写）", { exact: true }),
    ).toBeFocused();
    await form
      .getByLabel("节点标识（可选）", { exact: true })
      .fill("manual-http-proof");
    await form.getByLabel("IP 或域名", { exact: true }).fill("127.0.0.1");
    await form.getByLabel("代理端口", { exact: true }).fill(String(port));
    await form.getByLabel("上游用户名（可选）", { exact: true }).fill(username);
    const secret = form.getByLabel("上游密码（可选）", { exact: true });
    await secret.fill(password);
    await expect(secret).toHaveAttribute("type", "password");
    await page.setViewportSize({ width: 1440, height: 1100 });
    await page.screenshot({
      path: "/private/tmp/hps-manual-nodes-desktop.png",
      fullPage: true,
      animations: "disabled",
    });
    await page.setViewportSize({ width: 390, height: 844 });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: "/private/tmp/hps-manual-nodes-mobile.png",
      fullPage: true,
      animations: "disabled",
    });
    await form.getByRole("button", { name: "添加节点", exact: true }).click();
    await expect(page.getByRole("main").getByRole("status")).toContainText(
      "已添加静态节点 manual-http-proof",
    );
    await expect(page).toHaveURL(/search=manual-http-proof/);
    const row = page.getByRole("row", {
      name: "节点 manual-http-proof",
      exact: true,
    });
    await expect(row).toContainText(`http://127.0.0.1:${port}`);
    await expect(row.getByRole("cell", { name: "静态节点" })).toBeVisible();
    await page.reload();
    await expect(row).toBeVisible();
    await expect
      .poll(async () => {
        const inventory = await (
          await request.get("/api/v1/nodes?id=manual-http-proof")
        ).json();
        return inventory.nodes[0]?.eligible;
      })
      .toBe(true);
    const inventory = await (
      await request.get("/api/v1/nodes?id=manual-http-proof")
    ).json();
    expect(inventory.nodes[0].has_credentials).toBe(true);
    expect(JSON.stringify(inventory)).not.toContain(password);
    expect(JSON.stringify(inventory)).not.toContain(username);
    const inlet = await (
      await request.post("/api/v1/inbounds/browser-http/start")
    ).json();
    expect(
      await traffic(
        Number(inlet.listen.split(":").at(-1)),
        `http://127.0.0.1:${originPort}/business`,
      ),
    ).toBe("203.0.113.41\n");
    expect(authenticated).toBeGreaterThanOrEqual(2);
    expect(originRequests).toBeGreaterThanOrEqual(2);
    expect(leakedProxyAuth).toBe(false);
    const connections = await (
      await request.get("/api/v1/connections?state=all&node=manual-http-proof")
    ).json();
    expect(
      connections.records.some(
        (r: { choice: { node_id: string }; error_code: string | null }) =>
          r.choice?.node_id === "manual-http-proof" && !r.error_code,
      ),
    ).toBe(true);
  } finally {
    await reset(request);
    upstream.closeAllConnections();
    origin.closeAllConnections();
    await new Promise<void>((resolve) => upstream.close(() => resolve()));
    await new Promise<void>((resolve) => origin.close(() => resolve()));
  }
});

test("manual node URL, IPv6, expiry, duplicate and stale revisions keep credentials and existing nodes intact", async ({
  page,
  request,
}) => {
  try {
    await reset(request);
    await page.goto("/console/nodes.html");
    await page.getByRole("button", { name: "手动添加", exact: true }).click();
    const panel = page.getByRole("region", {
      name: "手动添加节点",
      exact: true,
    });
    const form = page.getByRole("form", { name: "单个代理节点" });
    const protocol = form.getByRole("combobox", {
      name: "代理协议",
      exact: true,
    });
    await protocol.selectOption("https");
    await expect(form.getByLabel("代理端口", { exact: true })).toHaveValue(
      "443",
    );
    await protocol.selectOption("socks5");
    await expect(form.getByLabel("代理端口", { exact: true })).toHaveValue(
      "1080",
    );
    await form.getByLabel("代理端口", { exact: true }).fill("9080");
    await protocol.selectOption("http");
    await expect(form.getByLabel("代理端口", { exact: true })).toHaveValue(
      "9080",
    );
    const url = panel.getByLabel("代理链接（可选，用于快速填写）", {
      exact: true,
    });
    await url.fill("vmess://unsupported");
    await panel.getByRole("button", { name: "填入表单", exact: true }).click();
    await expect(panel.getByRole("alert")).toContainText("有效的 HTTP");
    await url.fill("socks5h://manual-user:p%40ss%3A%2B@[::1]:1080");
    await panel.getByRole("button", { name: "填入表单", exact: true }).click();
    await expect(url).toHaveValue("");
    await expect(
      form.getByRole("combobox", { name: "代理协议", exact: true }),
    ).toHaveValue("socks5");
    await expect(form.getByLabel("IP 或域名", { exact: true })).toHaveValue(
      "::1",
    );
    const secret = form.getByLabel("上游密码（可选）", { exact: true });
    await expect(secret).toHaveValue("p@ss:+");
    await expect(secret).toHaveAttribute("type", "password");
    await form
      .getByRole("button", { name: "显示上游密码", exact: true })
      .click();
    await expect(secret).toHaveAttribute("type", "text");
    await form
      .getByRole("button", { name: "隐藏上游密码", exact: true })
      .click();
    await form
      .getByLabel("节点标识（可选）", { exact: true })
      .fill("manual-socks-proof");
    await form.getByLabel("添加后启用", { exact: true }).uncheck();
    const expiry = form.getByLabel("到期时间（可选，本地时间）", {
      exact: true,
    });
    await expiry.fill("2020-01-01T00:00");
    await form.getByRole("button", { name: "添加节点", exact: true }).click();
    await expect(panel.getByRole("alert")).toContainText(
      "到期时间必须晚于当前时间",
    );
    const future = await page.evaluate(() => {
      const d = new Date(Date.now() + 86_400_000);
      return new Date(d.getTime() - d.getTimezoneOffset() * 60_000)
        .toISOString()
        .slice(0, 16);
    });
    await expiry.fill(future);
    await form.getByRole("button", { name: "添加节点", exact: true }).click();
    await expect(page.getByRole("main").getByRole("status")).toContainText(
      "已添加静态节点 manual-socks-proof",
    );
    let inventory = await (await request.get("/api/v1/nodes")).json();
    expect(inventory.nodes[0]).toMatchObject({
      id: "manual-socks-proof",
      protocol: "socks5",
      host: "::1",
      port: 1080,
      has_credentials: true,
      configuration_enabled: false,
      eligible: false,
    });
    expect(inventory.nodes[0].expires_at_unix).toBeGreaterThan(
      Date.now() / 1000,
    );
    await page.getByRole("button", { name: "手动添加", exact: true }).click();
    await form
      .getByLabel("节点标识（可选）", { exact: true })
      .fill("manual-socks-proof");
    await form.getByLabel("IP 或域名", { exact: true }).fill("127.0.0.1");
    await form.getByRole("button", { name: "添加节点", exact: true }).click();
    await expect(panel.getByRole("alert")).toContainText("节点标识已存在");
    expect(
      (await (await request.get("/api/v1/nodes")).json()).summary.inventory,
    ).toBe(1);
    await form
      .getByLabel("节点标识（可选）", { exact: true })
      .fill("manual-cas-proof");
    const workspace = await (
      await request.get("/api/v1/config/workspace")
    ).json();
    const health = await (await request.get("/api/v1/health")).json();
    expect(
      (
        await request.post("/api/v1/health/save", {
          data: {
            configuration: health.configuration,
            expected_revision: workspace.desired_revision,
          },
        })
      ).ok(),
    ).toBe(true);
    await form.getByRole("button", { name: "添加节点", exact: true }).click();
    await expect(panel.getByRole("alert")).toContainText("已刷新版本");
    await expect(form.getByLabel("IP 或域名", { exact: true })).toHaveValue(
      "127.0.0.1",
    );
    await expect(
      form.getByRole("button", { name: "添加节点", exact: true }),
    ).toBeEnabled();
    await form.getByRole("button", { name: "添加节点", exact: true }).click();
    await expect(page.getByRole("main").getByRole("status")).toContainText(
      "已添加静态节点 manual-cas-proof",
    );
    inventory = await (await request.get("/api/v1/nodes")).json();
    expect(inventory.summary.inventory).toBe(2);
    expect(
      inventory.nodes.find((n: { id: string }) => n.id === "manual-socks-proof")
        .has_credentials,
    ).toBe(true);
    await page.getByRole("button", { name: "手动添加", exact: true }).click();
    await form
      .getByLabel("IP 或域名", { exact: true })
      .fill("proxy.example.test");
    await form.getByLabel("添加后启用", { exact: true }).uncheck();
    await form.getByRole("button", { name: "添加节点", exact: true }).click();
    await expect(page.getByRole("main").getByRole("status")).toContainText(
      /已添加静态节点 manual-\d+-\d+/,
    );
    inventory = await (await request.get("/api/v1/nodes")).json();
    expect(inventory.summary.inventory).toBe(3);
    expect(
      inventory.nodes.find(
        (n: { host: string }) => n.host === "proxy.example.test",
      ),
    ).toMatchObject({ has_credentials: false, configuration_enabled: false });
  } finally {
    await reset(request);
  }
});
