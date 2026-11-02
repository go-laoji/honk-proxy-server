import { test, expect } from "@playwright/test";
import { readFile } from "node:fs/promises";
test("rules import previews a versioned file, saves an unactivated candidate and exports no supplier secrets", async ({
  page,
  request,
}, testInfo) => {
  const workspace = await (
    await request.get("/api/v1/config/workspace")
  ).json();
  const effective = await (await request.get("/api/v1/config")).json();
  try {
    const prepared = await request.post("/api/v1/config/revisions", {
      data: {
        toml: `management_listen="${effective.management_listen}"\n[health]\nenabled=false\n[[proxy.upstreams]]\nid="private-supplier"\nprotocol="http"\nhost="127.0.0.1"\nport=9\nenabled=false\n[proxy.upstreams.credentials]\nusername="supplier"\npassword="rules-should-not-export-secret"\n`,
        expected_revision: workspace.desired_revision,
        note: "rule export scope",
      },
    });
    expect(prepared.ok()).toBe(true);
    const revision = await prepared.json();
    expect(
      (
        await request.post("/api/v1/config/activate", {
          data: {
            revision: revision.id,
            expected_revision: workspace.desired_revision,
          },
        })
      ).ok(),
    ).toBe(true);
    await page.goto("/console/rules.html");
    const panel = page.getByRole("region", { name: "规则批量导入导出" });
    await panel.getByRole("button", { name: "读取所看规则到编辑框" }).click();
    const editor = panel.getByRole("textbox", { name: "规则导入 JSON" });
    await expect(editor).toHaveValue(/format_version/);
    expect(await editor.inputValue()).not.toContain(
      "rules-should-not-export-secret",
    );
    const file = JSON.parse(await editor.inputValue());
    file.configuration.rules = [
      {
        id: "file-import",
        enabled: true,
        conditions: { domain_suffixes: ["example.test"] },
        action: { kind: "reject" },
      },
    ];
    await panel.getByLabel("选择规则 JSON").setInputFiles({
      name: "rules.json",
      mimeType: "application/json",
      buffer: Buffer.from(JSON.stringify(file)),
    });
    await panel.getByRole("button", { name: "预览规则导入" }).click();
    await expect(panel).toContainText("预览有效 · 1 条规则");
    await panel.getByRole("button", { name: "创建导入候选" }).click();
    await expect(panel.getByRole("status")).toContainText("尚未激活");
    const candidate = Number(new URL(page.url()).searchParams.get("revision"));
    expect(candidate).toBeGreaterThan(revision.id);
    const before = await (await request.get("/api/v1/rules")).json();
    expect(before.running_revision).toBe(revision.id);
    expect(before.configuration.rules).toHaveLength(0);
    const [download] = await Promise.all([
      page.waitForEvent("download"),
      panel.getByRole("button", { name: "导出规则文件" }).click(),
    ]);
    const path = testInfo.outputPath("rules-export.json");
    await download.saveAs(path);
    const exported = await readFile(path, "utf8");
    expect(exported).not.toContain("rules-should-not-export-secret");
    expect(JSON.parse(exported).configuration.rules[0].id).toBe("file-import");
    await page
      .getByRole("button", { name: "激活查看的候选", exact: true })
      .click();
    await expect
      .poll(
        async () =>
          (await (await request.get("/api/v1/rules")).json()).running_revision,
      )
      .toBe(candidate);
    const fresh = page.getByRole("region", { name: "规则批量导入导出" });
    await fresh
      .getByRole("textbox", { name: "规则导入 JSON" })
      .fill(JSON.stringify({ ...file, format_version: 99 }));
    await fresh.getByRole("button", { name: "预览规则导入" }).click();
    await expect(fresh.getByRole("alert")).toBeVisible();
    await expect(
      fresh.getByRole("button", { name: "创建导入候选" }),
    ).toBeDisabled();
    await page.setViewportSize({ width: 390, height: 844 });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: "/tmp/hps-rules-import-mobile.png",
      fullPage: true,
      animations: "disabled",
    });
  } finally {
    const current = await (
      await request.get("/api/v1/config/workspace")
    ).json();
    if (current.desired_revision !== 1)
      await request.post("/api/v1/config/rollback", {
        data: { revision: 1, expected_revision: current.desired_revision },
      });
  }
});
