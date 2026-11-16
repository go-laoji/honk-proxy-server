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
      [
        ["group-a", "http", true],
        ["group-b", "socks5", false],
        ["group-c", "http", true],
        ...Array.from({ length: 51 }, (_, n) => [
          `extra-${n.toString().padStart(2, "0")}`,
          "http",
          true,
        ]),
      ]
        .map(
          ([id, protocol, enabled]) =>
            `\n[[proxy.upstreams]]\nid="${id}"\nprotocol="${protocol}"\nhost="127.0.0.1"\nport=9\nenabled=${enabled}\n`,
        )
        .join("");
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
    const members = editor.getByRole("combobox", {
      name: "成员节点",
      exact: true,
    });
    const search = editor.getByRole("searchbox", {
      name: "搜索成员节点",
      exact: true,
    });
    await search.fill("group-");
    await members.selectOption("group-a");
    await expect(members.locator('option[value="group-a"]')).toHaveCount(0);
    await members.selectOption("group-b");
    await members.selectOption("group-c");
    await search.fill("extra-50");
    await members.selectOption("extra-50");
    await editor
      .getByRole("button", { name: "移除 extra-50", exact: true })
      .press("Enter");
    await editor
      .getByRole("combobox", { name: "选点策略", exact: true })
      .selectOption("fixed");
    await editor
      .getByRole("searchbox", { name: "搜索固定节点", exact: true })
      .fill("group-a");
    await editor
      .getByRole("combobox", { name: "固定节点", exact: true })
      .selectOption("group-a");
    await page.setViewportSize({ width: 390, height: 844 });
    expect(
      await page.evaluate(() => document.documentElement.scrollWidth),
    ).toBeLessThanOrEqual(390);
    await page.getByRole("region", { name: "分组编辑器" }).screenshot({
      path: "/private/tmp/hps-issue52-mobile-editor.png",
      animations: "disabled",
    });
    await page.setViewportSize({ width: 1280, height: 720 });
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
    await tags
      .getByRole("searchbox", { name: "搜索节点", exact: true })
      .fill("group-a");
    await tags
      .getByRole("combobox", { name: "节点", exact: true })
      .selectOption("group-a");
    await tags.getByLabel("标签", { exact: true }).fill("local\npreferred");
    await editor.getByRole("button", { name: "添加入口绑定" }).click();
    const bindings = editor.getByRole("group", {
      name: "入口绑定",
      exact: true,
    });
    await bindings
      .getByRole("combobox", { name: "入口", exact: true })
      .selectOption("browser-http");
    await bindings
      .getByRole("combobox", { name: "分组", exact: true })
      .selectOption("browser");
    await editor.getByRole("button", { name: "保存并应用分组" }).click();
    await expect(page.getByRole("main").getByRole("status")).toContainText(
      "分组配置已保存",
    );
    const saved = (await (await request.get("/api/v1/groups")).json())
      .configuration;
    expect(saved.node_tags["group-a"]).toEqual(["local", "preferred"]);
    expect(saved.inbound_groups).toEqual({ "browser-http": "browser" });
    expect(saved.groups[0].members).toEqual(["group-a", "group-b", "group-c"]);
    const pinnedInventory = await (
      await request.get("/api/v1/nodes?group=browser")
    ).json();
    expect(
      pinnedInventory.nodes.map((node: { id: string }) => node.id),
    ).toEqual(["group-a"]);
    await page.goto("/console/group-detail.html?id=browser");
    await expect(
      page.getByRole("heading", { name: "分组详情", exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole("combobox", { name: "分组", exact: true }),
    ).toHaveValue("browser");
    await page.goto("/console/groups.html");
    await page
      .getByRole("combobox", { name: "入口", exact: true })
      .selectOption("browser-http");
    await page.getByRole("button", { name: "模拟选点", exact: true }).click();
    await expect(page.getByLabel("选点结果", { exact: true })).toContainText(
      "选择 group-a",
    );
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
    const pin = page.getByRole("combobox", { name: "固定节点", exact: true });
    await pin.selectOption("group-b");
    await expect(pin.locator('option[value="extra-00"]')).toHaveCount(0);
    await page.getByRole("button", { name: "应用固定", exact: true }).click();
    await page.getByRole("button", { name: "模拟选点", exact: true }).click();
    await expect(result).toContainText("无可用节点");
    const fallback = page.getByRole("combobox", {
      name: "允许回退节点",
      exact: true,
    });
    await fallback.selectOption("group-c");
    await fallback.selectOption("group-a");
    await expect(fallback.locator('option[value="group-b"]')).toHaveCount(0);
    await page
      .getByRole("button", { name: "上移 group-a", exact: true })
      .press("Enter");
    await page.getByRole("button", { name: "应用固定", exact: true }).click();
    await expect
      .poll(
        async () =>
          (await (await request.get("/api/v1/groups")).json()).configuration
            .groups[0].fallback_nodes,
      )
      .toEqual(["group-a", "group-c"]);
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
    await page
      .getByRole("button", { name: "编辑选择策略", exact: true })
      .click();
    await editor
      .getByRole("button", { name: "移除入口绑定 1", exact: true })
      .click();
    await editor.getByRole("button", { name: "保存并应用分组" }).click();
    await expect(page.getByRole("main").getByRole("status")).toContainText(
      "分组配置已保存",
    );
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
