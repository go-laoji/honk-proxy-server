import { test, expect } from "@playwright/test";
test("standard subscription MPA persists imports, source inventory and nested Score explanations", async ({
  page,
  request,
}) => {
  await page.goto("/console/providers.html");
  const panel = page.getByLabel("标准订阅", { exact: true });
  await expect(panel).toContainText("短效供应商仍在下方管理");
  await panel
    .getByRole("button", { name: "新增标准订阅", exact: true })
    .click();
  const editor = panel.getByRole("form", { name: "订阅编辑器" });
  await editor.getByLabel("订阅 ID").fill("browser-sub");
  await editor
    .getByLabel("订阅 URL")
    .fill("https://example.test/sub?token=private-browser-secret");
  await panel
    .getByRole("button", { name: "保存并应用订阅", exact: true })
    .click();
  await expect(panel.getByRole("status")).toContainText("按版本应用");
  await panel
    .getByRole("button", { name: "导入订阅文档", exact: true })
    .click();
  await panel
    .getByRole("textbox", { name: /文档内容/ })
    .fill(
      "socks5://user:private-browser-password@127.0.0.1:1080#one\nhttp://127.0.0.1:8080#two",
    );
  await panel
    .getByRole("button", { name: "解析并替换该订阅快照", exact: true })
    .click();
  await expect(panel).toContainText("2 个快照节点");
  await expect(panel).not.toContainText("private-browser-password");
  await panel.getByRole("button", { name: "编辑订阅", exact: true }).click();
  await expect(editor.getByLabel("订阅 URL")).toHaveValue("[REDACTED]");
  await editor.getByLabel("显示名称").fill("测试订阅");
  await editor.getByRole("button", { name: "保存并应用订阅" }).click();
  await expect(
    panel.getByRole("heading", { name: "测试订阅 · browser-sub" }),
  ).toBeVisible();
  await expect(panel).toContainText("2 个快照节点");

  await expect(panel).not.toContainText("private-browser-secret");
  await page.reload();
  await expect(panel).toContainText("2 个快照节点");
  await panel.getByRole("link", { name: "查看来源节点", exact: true }).click();
  await expect(page).toHaveURL(/provider=subscription%3Abrowser-sub/);
  await expect(
    page.getByText("socks5://127.0.0.1:1080", { exact: true }).first(),
  ).toBeVisible();
  const sources = await (await request.get("/api/v1/subscriptions")).json();
  const group = {
    id: "child",
    include_all: false,
    members: [],
    providers: ["subscription:browser-sub"],
    groups: [],
    tags: [],
    protocols: ["socks5"],
    strategy: "round_robin",
  };
  let response = await request.post("/api/v1/groups/upsert", {
    data: { group, expected_revision: sources.running_revision },
  });
  expect(response.ok()).toBe(true);
  let applied = await response.json();
  response = await request.post("/api/v1/groups/upsert", {
    data: {
      group: {
        ...group,
        id: "parent",
        providers: [],
        protocols: [],
        groups: ["child"],
        strategy: "score",
      },
      expected_revision: applied.running_revision,
    },
  });
  expect(response.ok()).toBe(true);
  applied = await response.json();
  await page.goto("/console/groups.html?group=parent");
  await page.getByRole("button", { name: "模拟选点", exact: true }).click();
  const result = page.getByLabel("选点结果", { exact: true });
  await expect(result).toContainText("证据评分");
  await expect(result).toContainText("子组：child");
  await result
    .getByText(/Score .*分量与证据/)
    .first()
    .click();
  await expect(result).toContainText("缺少证据");
  await expect(result).toContainText("缺证据扣分 250");
  const policy = await (await request.get("/api/v1/groups")).json();
  policy.configuration.groups.find(
    (g: { id: string }) => g.id === "child",
  ).groups = ["parent"];
  response = await request.post("/api/v1/groups/save", {
    data: {
      configuration: policy.configuration,
      expected_revision: policy.running_revision,
    },
  });
  expect(response.status()).toBe(422);
  await page.goto("/console/providers.html");
  await panel
    .getByRole("button", { name: "导入订阅文档", exact: true })
    .click();
  await panel
    .getByRole("textbox", { name: /文档内容/ })
    .fill("ss://unsupported");
  await panel
    .getByRole("button", { name: "解析并替换该订阅快照", exact: true })
    .click();
  await expect(panel.getByRole("alert")).toBeVisible();
  await expect(panel).toContainText("2 个快照节点");
});
