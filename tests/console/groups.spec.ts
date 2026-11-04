import { test, expect } from "@playwright/test";

test("groups MPA edits, explains strict pins and UDP exclusions, rejects stale writes and deletes", async ({
  page,
  request,
}) => {
  try {
    const baseline = await (
      await request.get("/api/v1/config/revisions/1")
    ).json();
    const workspace = await (
      await request.get("/api/v1/config/workspace")
    ).json();
    const toml =
      baseline.toml.replace("upstreams = []\n", "") +
      '\n[[proxy.upstreams]]\nid="group-a"\nprotocol="http"\nhost="127.0.0.1"\nport=9\n\n[[proxy.upstreams]]\nid="group-b"\nprotocol="socks5"\nhost="127.0.0.1"\nport=9\nenabled=false\n';
    const revisionResponse = await request.post("/api/v1/config/revisions", {
      data: {
        toml,
        expected_revision: workspace.desired_revision,
        note: "group fixture",
      },
    });
    expect(revisionResponse.ok()).toBe(true);
    const revision = await revisionResponse.json();
    expect(
      (
        await request.post("/api/v1/config/activate", {
          data: {
            revision: revision.id,
            expected_revision: workspace.desired_revision,
            defer_until_restart: false,
          },
        })
      ).ok(),
    ).toBe(true);
    await page.goto("/console/groups.html");
    await expect(
      page.getByText("尚未配置分组，当前使用全池轮询。", { exact: true }),
    ).toBeVisible();
    await page.getByRole("button", { name: "新增分组", exact: true }).click();
    const editor = page.getByRole("form", { name: "分组配置" });
    await editor.getByLabel("分组 ID", { exact: true }).fill("browser");
    await editor
      .getByLabel("成员节点 ID", { exact: true })
      .fill("group-a\ngroup-b");
    await editor
      .getByRole("combobox", { name: "选点策略", exact: true })
      .selectOption("fixed");
    await editor.getByLabel("固定节点 ID", { exact: true }).fill("group-a");
    await page
      .getByRole("button", { name: "保存并应用分组", exact: true })
      .click();
    await expect(page.getByRole("main").getByRole("status")).toContainText(
      "分组配置已保存",
    );
    await page
      .getByRole("button", { name: "编辑选择策略", exact: true })
      .click();
    await editor.getByRole("button", { name: "添加节点标签" }).click();
    const tags = editor.getByRole("group", { name: "节点标签", exact: true });
    await tags.getByLabel("节点 ID", { exact: true }).fill("group-a");
    await tags.getByLabel("标签", { exact: true }).fill("local\npreferred");
    await editor.getByRole("button", { name: "保存并应用分组" }).click();
    await expect(page.getByRole("main").getByRole("status")).toContainText(
      "分组配置已保存",
    );
    const saved = (await (await request.get("/api/v1/groups")).json())
      .configuration;
    expect(saved.node_tags["group-a"]).toEqual(["local", "preferred"]);
    expect(saved.groups[0].members).toEqual(["group-a", "group-b"]);
    await page.goto("/console/group-detail.html?id=browser");
    await expect(
      page.getByRole("heading", { name: "分组详情", exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole("combobox", { name: "分组", exact: true }),
    ).toHaveValue("browser");
    await page.goto("/console/groups.html");
    await page.getByRole("button", { name: "选择此组", exact: true }).click();
    await page.getByRole("button", { name: "模拟选点", exact: true }).click();
    const result = page.getByLabel("选点结果", { exact: true });
    await expect(result).toContainText("选择 group-a");
    await expect(result).toContainText("手动固定");
    await page
      .getByRole("combobox", { name: "传输", exact: true })
      .selectOption("udp");
    await page.getByRole("button", { name: "模拟选点", exact: true }).click();
    await expect(result).toContainText("无可用节点");
    await expect(result.getByText(/仅支持 TCP/).first()).toBeVisible();
    await page
      .getByRole("combobox", { name: "传输", exact: true })
      .selectOption("tcp");
    await page
      .getByRole("textbox", { name: "固定节点 ID", exact: true })
      .fill("group-b");
    await page.getByRole("button", { name: "应用固定", exact: true }).click();
    await page.getByRole("button", { name: "模拟选点", exact: true }).click();
    await expect(result).toContainText("无可用节点");
    await page
      .getByRole("textbox", { name: "允许回退节点（逗号分隔）", exact: true })
      .fill("group-a");
    await page.getByRole("button", { name: "应用固定", exact: true }).click();
    await page.getByRole("button", { name: "模拟选点", exact: true }).click();
    await expect(result).toContainText("选择 group-a");
    await expect(result).toContainText("显式回退");
    await page.getByRole("button", { name: "取消固定", exact: true }).click();
    await expect(page.getByRole("main").getByRole("status")).toContainText(
      "固定已取消",
    );
    await page.getByRole("button", { name: "编辑", exact: true }).click();
    const snapshot = await (await request.get("/api/v1/groups")).json();
    snapshot.configuration.groups[0].strategy = "failover";
    expect(
      (
        await request.post("/api/v1/groups/save", {
          data: {
            configuration: snapshot.configuration,
            expected_revision: snapshot.running_revision,
          },
        })
      ).ok(),
    ).toBe(true);
    await page
      .getByRole("button", { name: "保存并应用分组", exact: true })
      .click();
    await expect(page.getByRole("alert")).toContainText(
      "configuration changed",
    );
    await page.getByRole("button", { name: "取消编辑", exact: true }).click();
    await page.getByRole("button", { name: "刷新分组", exact: true }).click();
    await page.setViewportSize({ width: 390, height: 844 });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: "/tmp/hps-t10-groups-mobile.png",
      fullPage: true,
      animations: "disabled",
    });
    await page.getByRole("button", { name: "删除", exact: true }).click();
    await expect(page.getByRole("main").getByRole("status")).toContainText(
      "分组已删除",
    );
    await expect(
      page.getByText("尚未配置分组，当前使用全池轮询。", { exact: true }),
    ).toBeVisible();
  } finally {
    const workspace = await (
      await request.get("/api/v1/config/workspace")
    ).json();
    await request.post("/api/v1/config/rollback", {
      data: { revision: 1, expected_revision: workspace.desired_revision },
    });
  }
});
