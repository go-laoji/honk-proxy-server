import { test, expect } from "@playwright/test";
import http from "node:http";
import type { AddressInfo } from "node:net";
type NodesOverview = {
  running_revision: number;
  summary: { inventory: number };
  nodes: { id: string; protocol: string; operation: { version: number } }[];
};

async function throughProxy(port: number): Promise<number> {
  return new Promise((resolve, reject) => {
    const req = http.get(
      {
        host: "127.0.0.1",
        port,
        path: "http://node-target.test/check",
        headers: { Host: "node-target.test" },
        agent: false,
      },
      (response) => {
        response.resume();
        response.on("end", () => resolve(response.statusCode ?? 0));
      },
    );
    req.on("error", reject);
    req.setTimeout(3000, () => req.destroy(new Error("proxy timeout")));
  });
}
test("nodes MPA previews imports, restores filters, gates actual proxy traffic and links facts", async ({
  page,
  request,
}) => {
  const upstream = http.createServer((_req, res) =>
    res.end("real-node-upstream"),
  );
  await new Promise<void>((resolve) =>
    upstream.listen(0, "127.0.0.1", resolve),
  );
  const port = (upstream.address() as AddressInfo).port;
  try {
    let workspace = await (
      await request.get("/api/v1/config/workspace")
    ).json();
    await request.post("/api/v1/config/rollback", {
      data: { revision: 1, expected_revision: workspace.desired_revision },
    });
    workspace = await (await request.get("/api/v1/config/workspace")).json();
    expect(
      (
        await request.post("/api/v1/providers/dashboard-source/save", {
          data: {
            expected_revision: workspace.desired_revision,
            configuration: {
              id: "dashboard-source",
              enabled: false,
              kind: "text",
              endpoint: "http://127.0.0.1:9/list",
              ttl_secs: 300,
              count: 1,
              count_parameter: "num",
              refill: {
                enabled: true,
                min_nodes: 1,
                target_nodes: 2,
                max_nodes: 3,
                batch_size: 1,
              },
            },
          },
        })
      ).ok(),
    ).toBe(true);
    await page.goto("/console/nodes.html");
    await page.locator("summary").filter({ hasText: "来源水位与异常" }).click();
    await expect(
      page.getByLabel("来源水位与异常", { exact: true }),
    ).toBeVisible();
    await expect(
      page.getByLabel("来源水位与异常", { exact: true }),
    ).toContainText("最小 1 / 目标 2 / 上限 3");
    await expect(
      page.getByText("没有符合筛选的节点；可手动添加、批量导入或调整筛选。", {
        exact: true,
      }),
    ).toBeVisible();
    await page.getByRole("button", { name: "批量导入", exact: true }).click();
    await page
      .getByRole("textbox", { name: "节点导入内容", exact: true })
      .fill(
        `http://supplier:private-test-value@127.0.0.1:${port}\nvmess://unsupported\nsocks5://[::1]:9`,
      );
    await page.getByRole("button", { name: "预览并校验", exact: true }).click();
    const preview = page.getByLabel("导入预览", { exact: true });
    await expect(preview).toContainText("有效 2 · 无效 1");
    await expect(preview).not.toContainText("private-test-value");
    await expect(
      preview.getByRole("button", {
        name: "确认导入并激活所选节点",
        exact: true,
      }),
    ).toBeDisabled();
    await page
      .getByRole("textbox", {
        name: "仅应用行号（可选，逗号分隔）",
        exact: true,
      })
      .fill("1,3");
    await page.getByRole("button", { name: "预览并校验", exact: true }).click();
    await preview
      .getByRole("button", { name: "确认导入并激活所选节点", exact: true })
      .click();
    await expect(page.getByRole("main").getByRole("status")).toContainText(
      "已导入并激活 2 个静态节点",
    );
    let inventory: NodesOverview = await (
      await request.get("/api/v1/nodes")
    ).json();
    const node = inventory.nodes.find((n) => n.protocol === "http")!;
    expect(inventory.summary.inventory).toBe(2);
    const table = page.getByRole("table", { name: "节点列表", exact: true });
    const rows = table.locator("tbody").getByRole("row");
    await expect(rows).toHaveCount(2);
    await page.getByText("高级筛选", { exact: true }).click();
    await page
      .getByRole("combobox", { name: "每页", exact: true })
      .selectOption("20");
    await page.getByRole("button", { name: "应用筛选", exact: true }).click();
    await expect(page).toHaveURL(/limit=20/);
    await page.goto("/console/nodes.html?limit=1");
    await expect(rows).toHaveCount(1);
    const firstRow = await rows.first().getAttribute("aria-label");
    await page.getByRole("button", { name: "下一页", exact: true }).click();
    await expect(page).toHaveURL(/offset=1/);
    await expect(rows.first()).not.toHaveAttribute("aria-label", firstRow!);
    await page.getByRole("button", { name: "上一页", exact: true }).click();
    await expect(rows.first()).toHaveAttribute("aria-label", firstRow!);
    await page.getByRole("button", { name: "重置", exact: true }).click();
    await expect(rows).toHaveCount(2);
    expect(
      (
        await request.post("/api/v1/groups/save", {
          data: {
            expected_revision: inventory.running_revision,
            configuration: {
              groups: [
                {
                  id: "browser-nodes",
                  members: [node.id],
                  strategy: "fixed",
                  pinned_node: node.id,
                },
              ],
              default_group: "browser-nodes",
            },
          },
        })
      ).ok(),
    ).toBe(true);
    const start = await request.post("/api/v1/inbounds/browser-http/start");
    expect(start.ok()).toBe(true);
    const inlet = await start.json();
    const listenPort = Number(inlet.listen.split(":").at(-1));
    expect(await throughProxy(listenPort)).toBe(200);
    await page.getByRole("button", { name: "刷新节点", exact: true }).click();
    const row = table.getByRole("row", {
      name: `节点 ${node.id}`,
      exact: true,
    });
    await expect(
      row.getByRole("button", { name: "禁用", exact: true }),
    ).toBeEnabled();
    await row.getByRole("button", { name: "禁用", exact: true }).click();
    await expect(page.getByRole("main").getByRole("status")).toContainText(
      "已禁用新连接",
    );
    expect(await throughProxy(listenPort)).toBe(503);
    await page
      .getByRole("combobox", { name: "状态", exact: true })
      .selectOption("disabled");
    await page.getByRole("button", { name: "应用筛选", exact: true }).click();
    await expect(page).toHaveURL(/state=disabled/);
    await page.reload();
    await expect(
      page.getByRole("combobox", { name: "状态", exact: true }),
    ).toHaveValue("disabled");
    await expect(rows).toHaveCount(1);
    await page.setViewportSize({ width: 390, height: 844 });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    expect(
      await page
        .getByRole("region", { name: "节点表格滚动区" })
        .evaluate((element) => element.scrollWidth > element.clientWidth),
    ).toBe(true);
    await page.screenshot({
      path: "/tmp/hps-t12-nodes-mobile.png",
      fullPage: true,
      animations: "disabled",
    });
    inventory = await (await request.get("/api/v1/nodes")).json();
    const current = inventory.nodes.find((n) => n.id === node.id)!;
    await request.post("/api/v1/nodes/action", {
      data: {
        id: node.id,
        action: "enable",
        expected_revision: inventory.running_revision,
        expected_control_version: current.operation.version,
      },
    });
    await row.getByRole("button", { name: "排空", exact: true }).click();
    await expect(page.getByRole("alert")).toContainText(
      "configuration changed",
    );
    await page.getByRole("button", { name: "重置", exact: true }).click();
    await expect(row).toBeVisible();
    await expect(
      row.getByRole("button", { name: "软删除", exact: true }),
    ).toHaveCount(0);
    await row.getByRole("button", { name: "排空", exact: true }).click();
    await expect(page.getByRole("main").getByRole("status")).toContainText(
      "已停止新连接",
    );
    expect(await throughProxy(listenPort)).toBe(503);
    await row.getByRole("button", { name: "解除限制", exact: true }).click();
    await expect(page.getByRole("main").getByRole("status")).toContainText(
      "已解除人工限制",
    );
    expect(await throughProxy(listenPort)).toBe(200);
    await row.getByRole("link", { name: node.id, exact: true }).click();
    await expect(page).toHaveURL(/node-detail\.html\?id=/);
    await expect(page.getByLabel("节点详情", { exact: true })).toContainText(
      "已配置（隐藏）",
    );
    await page
      .getByRole("link", { name: "健康检测与任务 →", exact: true })
      .click();
    await expect(
      page.getByRole("button", { name: new RegExp(node.id) }).first(),
    ).toBeVisible();
    await expect(
      page.getByText(`节点筛选：${node.id}`, { exact: false }),
    ).toBeVisible();
  } finally {
    await request.post("/api/v1/inbounds/browser-http/stop");
    const workspace = await (
      await request.get("/api/v1/config/workspace")
    ).json();
    await request.post("/api/v1/config/rollback", {
      data: { revision: 1, expected_revision: workspace.desired_revision },
    });
    upstream.closeAllConnections();
    await new Promise<void>((resolve) => upstream.close(() => resolve()));
  }
});

test("expired nodes only occupy pagination when explicitly requested", async ({
  page,
  request,
  baseURL,
}) => {
  const workspace = await (
    await request.get("/api/v1/config/workspace")
  ).json();
  const toml = `management_listen = "${new URL(baseURL!).host}"

[health]
enabled = false

[[proxy.upstreams]]
id = "expired-layout-proof"
protocol = "http"
host = "127.0.0.1"
port = 9
expires_at_unix = 1

[[proxy.upstreams]]
id = "current-layout-proof"
protocol = "http"
host = "127.0.0.1"
port = 9
`;
  const saved = await request.post("/api/v1/config/revisions", {
    data: { toml, expected_revision: workspace.desired_revision },
  });
  expect(saved.ok()).toBe(true);
  const revision = await saved.json();
  try {
    const applied = await request.post("/api/v1/config/activate", {
      data: {
        revision: revision.id,
        expected_revision: workspace.desired_revision,
      },
    });
    expect(applied.ok()).toBe(true);
    expect(await applied.json()).toMatchObject({ state: "applied" });
    await page.goto("/console/nodes.html?limit=1&sort=expiry_asc");
    const table = page.getByRole("table", { name: "节点列表" });
    await expect(
      table.getByRole("row", { name: "节点 current-layout-proof" }),
    ).toBeVisible();
    await expect(table.getByText("expired-layout-proof")).toHaveCount(0);
    await expect(
      page.getByRole("button", { name: "下一页", exact: true }),
    ).toBeDisabled();
    await page
      .getByRole("combobox", { name: "状态", exact: true })
      .selectOption("expired");
    await page.getByRole("button", { name: "应用筛选", exact: true }).click();
    await expect(
      table.getByRole("row", { name: "节点 expired-layout-proof" }),
    ).toBeVisible();
    await expect(table.getByText("current-layout-proof")).toHaveCount(0);
    await table.getByRole("link", { name: "expired-layout-proof" }).click();
    await expect(
      page.getByRole("region", { name: "节点详情", exact: true }),
    ).toContainText("expired-layout-proof");
  } finally {
    const head = await (await request.get("/api/v1/config/workspace")).json();
    await request.post("/api/v1/config/rollback", {
      data: {
        revision: workspace.running_revision,
        expected_revision: head.desired_revision,
      },
    });
  }
});
