import { test, expect } from "@playwright/test";
import http from "node:http";
import type { AddressInfo } from "node:net";
import type { NodesOverview } from "../../web/console/src/nodes";

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
    await expect(
      page.getByLabel("来源水位与异常", { exact: true }),
    ).toContainText("最小 1 / 目标 2 / 上限 3");
    await expect(
      page.getByText("没有符合筛选的节点；可调整条件或导入节点。", {
        exact: true,
      }),
    ).toBeVisible();
    await page
      .getByRole("button", { name: "添加 / 导入", exact: true })
      .click();
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
    await expect(page.getByRole("status")).toContainText(
      "已导入并激活 2 个静态节点",
    );
    let inventory: NodesOverview = await (
      await request.get("/api/v1/nodes")
    ).json();
    const node = inventory.nodes.find((n) => n.protocol === "http")!;
    expect(inventory.summary.inventory).toBe(2);
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
    const card = page.getByRole("article", {
      name: `节点 ${node.id}`,
      exact: true,
    });
    await expect(
      card.getByRole("button", { name: "禁用", exact: true }),
    ).toBeEnabled();
    await card.getByRole("button", { name: "禁用", exact: true }).click();
    await expect(page.getByRole("status")).toContainText("已禁用新连接");
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
    await expect(
      page.getByLabel("节点列表", { exact: true }).getByRole("article"),
    ).toHaveCount(1);
    await page.setViewportSize({ width: 390, height: 844 });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
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
    await card.getByRole("button", { name: "排空", exact: true }).click();
    await expect(page.getByRole("alert")).toContainText(
      "configuration changed",
    );
    await page.getByRole("button", { name: "重置", exact: true }).click();
    await expect(card).toBeVisible();
    await card.getByRole("button", { name: "软删除", exact: true }).click();
    await expect(page.getByRole("status")).toContainText("已软删除");
    expect(await throughProxy(listenPort)).toBe(503);
    await card.getByRole("button", { name: "解除限制", exact: true }).click();
    await expect(page.getByRole("status")).toContainText("已解除人工限制");
    expect(await throughProxy(listenPort)).toBe(200);
    await card.getByRole("link", { name: node.id, exact: true }).click();
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
