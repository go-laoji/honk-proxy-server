import { test, expect } from "@playwright/test";
import { createSocket } from "node:dgram";
import type { AddressInfo } from "node:net";

test("DNS MPA resolves through real UDP, shows TTL cache/context, clears and detects a stale editor", async ({
  page,
  request,
}) => {
  const upstream = createSocket("udp4");
  let count = 0;
  upstream.on("message", (query, peer) => {
    count++;
    const answer = Buffer.from([
      0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 0, 30, 0, 4, 203, 0, 113, 9,
    ]);
    const reply = Buffer.concat([query, answer]);
    reply.writeUInt16BE(0x8180, 2);
    reply.writeUInt16BE(1, 6);
    upstream.send(reply, peer.port, peer.address);
  });
  await new Promise<void>((resolve) => upstream.bind(0, "127.0.0.1", resolve));
  try {
    await page.setViewportSize({ width: 1440, height: 1100 });
    await page.goto("/console/dns.html");
    await expect(
      page.getByRole("heading", { name: "DNS 上游与路由" }),
    ).toBeVisible();
    await expect(
      page.getByText("系统解析 · 无 wire TTL，结果不缓存"),
    ).toBeVisible();
    await page
      .getByRole("button", { name: "编辑 DNS 策略", exact: true })
      .click();
    const editor = page.getByRole("form", { name: "DNS 策略表单" });
    await editor.getByRole("textbox", { name: "上游 ID" }).fill("udp-fixture");
    await editor
      .getByRole("combobox", { name: "解析协议" })
      .selectOption("udp");
    await editor
      .getByRole("textbox", { name: /上游 IP 与端口/ })
      .fill(`127.0.0.1:${(upstream.address() as AddressInfo).port}`);
    await editor
      .getByRole("combobox", { name: "默认上游" })
      .selectOption("udp-fixture");
    await page
      .getByRole("button", { name: "保存并应用 DNS", exact: true })
      .click();
    await expect(page.getByRole("main").getByRole("status")).toContainText(
      "DNS 策略已保存",
    );
    await page
      .getByRole("textbox", { name: "查询域名", exact: true })
      .fill("console.example");
    await page.getByRole("button", { name: "执行查询", exact: true }).click();
    const diagnostic = page.getByRole("region", { name: "DNS 查询诊断" });
    await expect(diagnostic.getByText(/IN A 203.0.113.9/)).toBeVisible();
    expect(count).toBe(1);
    await page.getByRole("button", { name: "执行查询", exact: true }).click();
    await expect(diagnostic.getByText(/cache · UDP/)).toBeVisible();
    expect(count).toBe(1);
    const context = page.getByRole("region", { name: "DNS 域名上下文" });
    await context
      .getByRole("textbox", { name: "IPv4 / IPv6", exact: true })
      .fill("203.0.113.9");
    await context
      .getByRole("button", { name: "查询上下文", exact: true })
      .click();
    await expect(context).toContainText("剩余 TTL");
    await expect(
      context.getByText("console.example", { exact: true }),
    ).toBeVisible();
    await page
      .getByRole("textbox", { name: "清理域名（留空清理全部）", exact: true })
      .fill("console.example");
    await page.getByRole("button", { name: "清理缓存", exact: true }).click();
    await expect(page.getByRole("main").getByRole("status")).toContainText(
      "已清理 1 条缓存",
    );
    await expect(page.getByText("当前页无缓存", { exact: true })).toBeVisible();
    await page
      .getByRole("button", { name: "编辑 DNS 策略", exact: true })
      .click();
    const snapshot = await (await request.get("/api/v1/dns")).json();
    snapshot.configuration.timeout_ms = 3000;
    expect(
      (
        await request.post("/api/v1/dns/save", {
          data: {
            configuration: snapshot.configuration,
            expected_revision: snapshot.running_revision,
          },
        })
      ).ok(),
    ).toBe(true);
    await page
      .getByRole("button", { name: "保存并应用 DNS", exact: true })
      .click();
    await expect(page.getByRole("alert")).toContainText(
      "configuration changed",
    );
    await page.getByRole("button", { name: "取消编辑", exact: true }).click();
    await page
      .getByRole("button", { name: "编辑 DNS 策略", exact: true })
      .click();
    await editor.getByText("监听、缓存与 Linux 捕获", { exact: true }).click();
    await expect(
      editor.getByRole("spinbutton", { name: "解析超时（毫秒）" }),
    ).toHaveValue("3000");
    await page.getByRole("button", { name: "取消编辑", exact: true }).click();
    await page.setViewportSize({ width: 390, height: 844 });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: "/tmp/hps-t09-dns-mobile.png",
      fullPage: true,
      animations: "disabled",
    });
  } finally {
    const workspace = await (
      await request.get("/api/v1/config/workspace")
    ).json();
    if (workspace.desired_revision !== 1)
      await request.post("/api/v1/config/rollback", {
        data: { revision: 1, expected_revision: workspace.desired_revision },
      });
    upstream.close();
  }
});
