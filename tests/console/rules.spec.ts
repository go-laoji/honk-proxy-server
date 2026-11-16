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
    await action.selectOption("node");
    const targetNode = editor.getByRole("combobox", {
      name: "目标节点",
      exact: true,
    });
    await expect(targetNode).toHaveValue("");
    await expect(targetNode).toHaveJSProperty("required", true);
    await action.selectOption("direct");
    await page.getByRole("button", { name: "保存为候选", exact: true }).click();
    await expect(page.getByRole("main").getByRole("status")).toContainText(
      "需要显式激活",
    );
    expect((await throughProxy(proxy, target)).status).toBe(503);
    const context = page.getByRole("form", { name: "规则模拟条件" });
    await context.getByLabel("目标域名或 IP").fill("127.0.0.1");
    await context.getByLabel("目标端口").fill(String(port));
    await context
      .getByRole("combobox", { name: "入口", exact: true })
      .selectOption("browser-http");
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

test("quick rules from a group and a connection route new requests to different groups", async ({
  page,
  request,
}) => {
  const upstreams = await Promise.all(
    ["alpha", "beta"].map(async (name) => {
      const server = createServer((_req, res) => {
        res.setHeader("Connection", "close");
        res.end(name);
      });
      await new Promise<void>((resolve) =>
        server.listen(0, "127.0.0.1", resolve),
      );
      return { server, name, port: (server.address() as AddressInfo).port };
    }),
  );
  const workspace = async () =>
    (await request.get("/api/v1/config/workspace")).json();
  const initial = await workspace();
  try {
    const effective = await (await request.get("/api/v1/config")).json();
    const toml =
      `management_listen="${effective.management_listen}"\n[health]\nenabled=false\n[[proxy.inbounds]]\nid="browser-http"\nprotocol="http"\nlisten="127.0.0.1:0"\nenabled=true\n[proxy.routing.default_action]\nkind="reject"\n` +
      upstreams
        .map(
          ({ name, port }) => `
[[proxy.upstreams]]
id="node-${name}"
protocol="http"
host="127.0.0.1"
port=${port}
[[proxy.selection.groups]]
id="${name}"
members=["node-${name}"]
`,
        )
        .join("");
    const created = await request.post("/api/v1/config/revisions", {
      data: {
        toml,
        expected_revision: initial.desired_revision,
        note: "group routing browser fixture",
      },
    });
    expect(created.ok()).toBe(true);
    const { id: revision } = await created.json();
    expect(
      (
        await request.post("/api/v1/config/activate", {
          data: { revision, expected_revision: initial.desired_revision },
        })
      ).ok(),
    ).toBe(true);
    const { inbounds } = await (await request.get("/api/v1/inbounds")).json();
    const proxy: string = inbounds.find(
      (inbound: { id: string }) => inbound.id === "browser-http",
    ).listen;
    const video = "http://video.example.test/";
    const api = "http://api.example.test/";
    const quick = page.getByRole("form", { name: "快速分流规则", exact: true });
    const simulation = page.getByRole("form", {
      name: "规则模拟条件",
      exact: true,
    });
    const result = page.getByLabel("规则模拟结果", { exact: true });
    async function saveCandidate() {
      await quick.getByRole("button", { name: "保存分流候选" }).click();
      await expect(quick).toHaveCount(0);
      await expect(page.getByRole("main").getByRole("status")).toContainText(
        "需要显式激活",
      );
    }
    async function activate() {
      await page
        .getByRole("button", { name: "激活查看的候选", exact: true })
        .click();
      await expect(page.getByRole("main").getByRole("status")).toContainText(
        "激活结果 applied",
      );
    }
    await page.goto("/console/groups.html");
    await page
      .getByRole("article")
      .filter({
        has: page.getByRole("heading", { name: "alpha", exact: true }),
      })
      .getByRole("link", { name: "配置分流" })
      .click();
    await expect(quick.getByLabel("目标代理组")).toHaveValue("alpha");
    await page.setViewportSize({ width: 390, height: 844 });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await quick.getByLabel("匹配方式").selectOption("domain_suffixes");
    await quick.getByLabel("匹配目标").fill(".example.test");
    await saveCandidate();
    expect((await throughProxy(proxy, video)).status).toBe(403);
    await expect(simulation.getByLabel("目标域名或 IP")).toHaveValue(
      "example.test",
    );
    await simulation.getByRole("button", { name: "执行规则模拟" }).click();
    await expect(result).toContainText("分组 alpha");
    await expect(result).toContainText("选择 node-alpha");
    await activate();
    expect(await throughProxy(proxy, video)).toEqual({
      status: 200,
      body: "alpha",
    });

    const connections = await (
      await request.get("/api/v1/connections?state=all&node=node-alpha")
    ).json();
    const connection = connections.records.find(
      (entry: { selection?: { selected?: string } }) =>
        entry.selection?.selected === "node-alpha",
    );
    expect(connection).toBeTruthy();
    await page.goto(
      `/console/connections.html?state=all&detail=${encodeURIComponent(connection.id)}`,
    );
    await page.getByRole("link", { name: "按此目标建规则" }).click();
    await expect(quick.getByLabel("匹配目标")).toHaveValue(
      "video.example.test",
    );
    await expect(quick.getByLabel("目标端口")).toHaveValue("80");
    await expect(quick.getByLabel("传输协议")).toHaveValue("tcp");
    await quick.getByLabel("目标代理组").selectOption("beta");
    await quick.getByLabel("规则位置").selectOption("first");
    await saveCandidate();
    expect(await throughProxy(proxy, video)).toEqual({
      status: 200,
      body: "alpha",
    });
    await simulation.getByRole("button", { name: "执行规则模拟" }).click();
    await expect(result).toContainText("命中 to-beta · 分组 beta");
    await activate();
    expect(await throughProxy(proxy, video)).toEqual({
      status: 200,
      body: "beta",
    });
    expect(await throughProxy(proxy, api)).toEqual({
      status: 200,
      body: "alpha",
    });

    await page.getByRole("button", { name: "按目标分流", exact: true }).click();
    await quick.getByLabel("匹配方式").selectOption("destination_cidrs");
    await quick.getByLabel("匹配目标").fill("[2001:db8::1]");
    await quick.getByLabel("目标代理组").selectOption("beta");
    await saveCandidate();
    await expect(page.getByRole("region", { name: "有序规则" })).toContainText(
      "2001:db8::1/128",
    );
    await activate();
    expect(await throughProxy(proxy, "http://[2001:db8::1]/")).toEqual({
      status: 200,
      body: "beta",
    });

    await page.getByRole("button", { name: "编辑规则策略" }).click();
    const editor = page.getByRole("form", { name: "规则配置" });
    await editor.getByLabel("流量动作").selectOption("node");
    await editor.getByLabel("目标节点").selectOption("node-alpha");
    await editor.getByLabel("流量动作").selectOption("group");
    await expect(editor.getByLabel("目标代理组")).toHaveValue("");
    await editor.getByLabel("目标代理组").selectOption("alpha");
    await editor.getByRole("button", { name: "保存为候选" }).click();
    await activate();
    expect(await throughProxy(proxy, "http://unmatched.test/")).toEqual({
      status: 200,
      body: "alpha",
    });
  } finally {
    const latest = await workspace();
    await request.post("/api/v1/config/rollback", {
      data: {
        revision: initial.desired_revision,
        expected_revision: latest.desired_revision,
      },
    });
    await Promise.all(
      upstreams.map(
        ({ server }) =>
          new Promise<void>((resolve) => server.close(() => resolve())),
      ),
    );
  }
});
