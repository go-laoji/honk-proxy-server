import { test, expect } from "@playwright/test";
import {
  createServer,
  request as httpRequest,
  type ServerResponse,
} from "node:http";
import type { AddressInfo } from "node:net";

test("health MPA keeps probes across navigation, recovers lost acknowledgment and applies policies with revision checks", async ({
  page,
  request,
}) => {
  let released = false;
  const waiting: ServerResponse[] = [];
  const origin = createServer((_req, res) => {
    if (released) {
      res.writeHead(200);
      res.end("203.0.113.10\n");
    } else waiting.push(res);
  });
  const proxy = () =>
    createServer((req, res) => {
      const target = new URL(req.url!);
      const outgoing = httpRequest(
        {
          hostname: target.hostname,
          port: target.port,
          path: target.pathname,
          method: req.method,
          headers: req.headers,
        },
        (incoming) => {
          res.writeHead(incoming.statusCode!, incoming.headers);
          incoming.pipe(res);
        },
      );
      outgoing.on("error", () => {
        res.writeHead(502);
        res.end();
      });
      req.pipe(outgoing);
    });
  const proxies = [proxy(), proxy()];
  const servers = [origin, ...proxies];
  await Promise.all(
    servers.map(
      (server) =>
        new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve)),
    ),
  );
  const port = (server: typeof origin) =>
    (server.address() as AddressInfo).port;
  try {
    const current = await (
      await request.get("/api/v1/config/workspace")
    ).json();
    const effective = await (await request.get("/api/v1/config")).json();
    const body = `management_listen="${effective.management_listen}"\ninstance_name="health-console-test"\n[health]\nenabled=true\nmax_parallel=1\ntimeout_ms=10000\ninterval_secs=60\nmax_staleness_secs=180\ndaily_probe_budget=20\n[[health.profiles]]\ntransport="tcp"\nfamily="ipv4"\nurl="http://127.0.0.1:${port(origin)}/health"\ncapture_exit_ip=true\n[[proxy.inbounds]]\nid="browser-http"\nprotocol="http"\nlisten="127.0.0.1:0"\nenabled=false\n${proxies.map((server, index) => `[[proxy.upstreams]]\nid="probe-node-${index + 1}"\nprotocol="http"\nhost="127.0.0.1"\nport=${port(server)}\n`).join("")}`;
    const revisionResponse = await request.post("/api/v1/config/revisions", {
      data: {
        toml: body,
        expected_revision: current.desired_revision,
        note: "health console proof",
      },
    });
    expect(revisionResponse.ok()).toBe(true);
    const revision = await revisionResponse.json();
    const activation = await request.post("/api/v1/config/activate", {
      data: {
        revision: revision.id,
        expected_revision: current.desired_revision,
      },
    });
    expect(activation.ok()).toBe(true);
    expect((await activation.json()).state).toBe("applied");
    await page.setViewportSize({ width: 1440, height: 1100 });
    await page.goto("/console/health.html");
    await expect(
      page.getByRole("heading", { name: "节点准入与能力" }),
    ).toBeVisible();
    await expect.poll(() => waiting.length).toBe(1);
    await page.getByRole("checkbox", { name: "选择节点 probe-node-1" }).check();
    await page
      .getByRole("button", { name: "检测所选 1 个", exact: true })
      .click();
    await expect(page.getByRole("main").getByRole("status")).toContainText(
      "已受理",
    );
    await expect(page.getByText("合并在途 1", { exact: false })).toBeVisible();
    await page.getByRole("link", { name: "运行概览", exact: true }).click();
    await expect(page).toHaveURL(/overview\.html$/);
    await page.getByRole("link", { name: "健康检查", exact: true }).click();
    await expect(page).toHaveURL(/health\.html$/);
    released = true;
    for (const response of waiting) {
      response.writeHead(200);
      response.end("203.0.113.10\n");
    }
    await expect
      .poll(async () => {
        const value = await (await request.get("/api/v1/health")).json();
        return value.nodes.filter(
          (node: { eligible: boolean }) => node.eligible,
        ).length;
      })
      .toBe(2);
    await expect(
      page.getByText("2 / 2", { exact: true }).first(),
    ).toBeVisible();
    await page.getByRole("button", { name: /^probe-node-1 ·/ }).click();
    await expect(
      page
        .getByRole("region", { name: "节点健康状态" })
        .getByText("TCP / IPV4", { exact: true }),
    ).toBeVisible();
    await expect(page.getByText("UDP / IPV4", { exact: true })).toBeVisible();
    await expect(page.getByText("暂不支持", { exact: true })).toHaveCount(2);
    await page
      .getByRole("button", { name: "编辑检测策略", exact: true })
      .click();
    const editor = page.getByRole("form", { name: "检测策略表单" });
    const interval = editor.getByRole("spinbutton", { name: "检测间隔（秒）" });
    await interval.fill("30");
    const policy = (await (await request.get("/api/v1/health?limit=1")).json())
      .configuration;
    policy.interval_secs = 30;
    const workspace = await (
      await request.get("/api/v1/config/workspace")
    ).json();
    expect(
      (
        await request.post("/api/v1/health/save", {
          data: {
            configuration: policy,
            expected_revision: workspace.desired_revision,
          },
        })
      ).ok(),
    ).toBe(true);
    await page
      .getByRole("button", { name: "保存并应用策略", exact: true })
      .click();
    await expect(page.getByRole("alert")).toContainText(
      "configuration changed",
    );
    await page.getByRole("button", { name: "取消编辑", exact: true }).click();
    await page
      .getByRole("button", { name: "编辑检测策略", exact: true })
      .click();
    await expect(interval).toHaveValue("30");
    await page
      .getByRole("button", { name: "保存并应用策略", exact: true })
      .click();
    await expect(page.getByRole("main").getByRole("status")).toContainText(
      "检测策略已保存",
    );
    // The actual POST reaches the daemon; only its acknowledgment is lost.
    await page.route("**/api/v1/health/probe", async (route) => {
      await route.fetch();
      await route.abort("failed");
    });
    await page
      .getByRole("button", { name: "单个检测", exact: true })
      .first()
      .click();
    await expect(
      page.getByRole("button", { name: "恢复上次检测" }),
    ).toBeVisible();
    const before = await (await request.get("/api/v1/health")).json();
    await page.unroute("**/api/v1/health/probe");
    await page.reload();
    await page.getByRole("button", { name: "恢复上次检测" }).click();
    await expect(page.getByRole("main").getByRole("status")).toContainText(
      "已受理",
    );
    const after = await (await request.get("/api/v1/health")).json();
    expect(after.used_probes).toBe(before.used_probes);
    const history = page.getByRole("region", { name: "健康历史与出口画像" });
    await history.getByRole("button", { name: "刷新历史" }).click();
    await expect(
      history.getByRole("cell").filter({ hasText: "203.0.113.10" }).first(),
    ).toBeVisible();
    await expect(history).toContainText("探测预算");
    await expect(history).not.toContainText("1970/");
    await expect(history.getByRole("img").first()).toBeVisible();
    await history
      .getByRole("combobox", { name: "健康历史窗口" })
      .selectOption("300");
    await expect(page).toHaveURL(/history_window=300/);
    await page.reload();
    await expect(
      history.getByRole("combobox", { name: "健康历史窗口" }),
    ).toHaveValue("300");
    await page.setViewportSize({ width: 390, height: 844 });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: "/tmp/hps-health-console.png",
      fullPage: true,
      animations: "disabled",
    });
  } finally {
    released = true;
    for (const response of waiting)
      if (!response.writableEnded) {
        response.writeHead(204);
        response.end();
      }
    const workspace = await (
      await request.get("/api/v1/config/workspace")
    ).json();
    if (workspace.desired_revision !== 1)
      await request.post("/api/v1/config/rollback", {
        data: { revision: 1, expected_revision: workspace.desired_revision },
      });
    for (const server of servers) {
      server.closeAllConnections();
      await new Promise<void>((resolve) => server.close(() => resolve()));
    }
  }
});
