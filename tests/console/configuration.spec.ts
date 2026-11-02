import { test, expect } from "@playwright/test";
import { readFile } from "node:fs/promises";

test("shared config draft conflicts, activation, redacted export, pending restart and rollback use actual APIs", async ({
  page,
  request,
}, testInfo) => {
  const peer = await page.context().newPage();
  try {
    await Promise.all([
      page.goto("/console/config.html"),
      peer.goto("/console/config.html"),
    ]);
    const editor = page.getByRole("textbox", {
      name: "配置 TOML",
      exact: true,
    });
    const peerEditor = peer.getByRole("textbox", {
      name: "配置 TOML",
      exact: true,
    });
    await expect(editor).toContainText("instance_name");
    await expect(peerEditor).toContainText("instance_name");
    const initial = await editor.inputValue();
    const candidate =
      initial
        .replace(
          'instance_name = "honk-proxy-server"',
          'instance_name = "console-config-instance"',
        )
        .replace("upstreams = []\n", "") +
      '\n[[proxy.upstreams]]\nid="console-fixture"\nprotocol="http"\nhost="127.0.0.1"\nport=18080\nenabled=false\n[proxy.upstreams.credentials]\nusername="supplier"\npassword="console-private-password"\n';
    await editor.fill(candidate);
    await page.getByRole("button", { name: "保存草稿", exact: true }).click();
    await expect(page.getByRole("status")).toContainText("共享草稿已保存");
    await peerEditor.fill(
      initial.replace(
        'instance_name = "honk-proxy-server"',
        'instance_name = "stale-tab"',
      ),
    );
    await peer.getByRole("button", { name: "保存草稿", exact: true }).click();
    await expect(peer.getByRole("alert")).toContainText(
      "configuration changed",
    );
    await page.reload();
    await expect(editor).toHaveValue(/\[REDACTED\]/);
    await page.getByRole("button", { name: "校验并预览", exact: true }).click();
    await expect(page.getByRole("status")).toContainText("校验通过");
    await expect(
      page.getByRole("button", { name: "保存版本", exact: true }),
    ).toBeEnabled();
    await page.getByRole("button", { name: "保存版本", exact: true }).click();
    await expect(page.getByRole("status")).toContainText("已保存，尚未激活");
    let workspace = await (
      await request.get("/api/v1/config/workspace")
    ).json();
    const revision: number = workspace.revisions[0].id;
    await page
      .getByRole("button", { name: `激活 r${revision}`, exact: true })
      .click();
    await expect(page.getByRole("status")).toContainText(
      "已应用到实际运行代际",
    );
    expect(
      (await (await request.get("/api/v1/status")).json()).instance_name,
    ).toBe("console-config-instance");
    const [download] = await Promise.all([
      page.waitForEvent("download"),
      page.getByRole("button", { name: "导出脱敏配置", exact: true }).click(),
    ]);
    const exportedPath = testInfo.outputPath("redacted-export.toml");
    await download.saveAs(exportedPath);
    const exported = await readFile(exportedPath, "utf8");
    expect(exported).toContain("format_version=1");
    expect(exported).toContain("[REDACTED]");
    expect(exported).not.toContain("console-private-password");
    const pending = await (
      await request.post("/api/v1/config/revisions", {
        data: {
          toml: exported.replace('log_filter = "info"', 'log_filter = "warn"'),
          expected_revision: revision,
          note: "restart-only",
        },
      })
    ).json();
    await page.reload();
    await expect(
      page.getByRole("button", { name: `激活版本 ${pending.id}`, exact: true }),
    ).toBeVisible();
    await page
      .getByRole("button", { name: `激活版本 ${pending.id}`, exact: true })
      .click();
    await expect(page.getByRole("status")).toContainText("需重启 Daemon");
    workspace = await (await request.get("/api/v1/config/workspace")).json();
    expect(workspace.desired_revision).toBe(pending.id);
    expect(workspace.running_revision).toBe(revision);
    expect(workspace.pending_restart).toContain("log_filter");
    await page.getByRole("button", { name: "回滚版本 1", exact: true }).click();
    await expect(page.getByRole("status")).toContainText(
      "已应用到实际运行代际",
    );
    expect(
      (await (await request.get("/api/v1/status")).json()).instance_name,
    ).toBe("honk-proxy-server");
    await page.setViewportSize({ width: 390, height: 844 });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: "/tmp/hps-configuration-console.png",
      fullPage: true,
      animations: "disabled",
    });
    expect((await page.context().cookies()).length).toBe(0);
  } finally {
    const current = await (
      await request.get("/api/v1/config/workspace")
    ).json();
    if (current.desired_revision !== 1) {
      await request.post("/api/v1/config/rollback", {
        data: { revision: 1, expected_revision: current.desired_revision },
      });
    }
    await peer.close();
  }
});
