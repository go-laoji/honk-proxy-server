import { test, expect } from "@playwright/test";
import { request as proxyRequest } from "node:http";

test("shared observation pages show real business counters, bounded history and URL windows without credentials", async ({
  page,
  request,
}) => {
  const current = await (await request.get("/api/v1/config/workspace")).json();
  const effective = await (await request.get("/api/v1/config")).json();
  try {
    const prepared = await request.post("/api/v1/config/revisions", {
      data: {
        toml: `management_listen="${effective.management_listen}"\n[health]\nenabled=false\n[[proxy.inbounds]]\nid="history"\nprotocol="http"\nlisten="127.0.0.1:0"\n[proxy.routing.default_action]\nkind="reject"\n`,
        expected_revision: current.desired_revision,
        note: "observation test",
      },
    });
    expect(prepared.ok()).toBe(true);
    const revision = await prepared.json();
    expect(
      (
        await request.post("/api/v1/config/activate", {
          data: {
            revision: revision.id,
            expected_revision: current.desired_revision,
          },
        })
      ).ok(),
    ).toBe(true);
    const inlet = (await (await request.get("/api/v1/inbounds")).json())
      .inbounds[0].listen as string;
    const [host, port] = inlet.split(":");
    await new Promise<void>((resolve, reject) => {
      const outgoing = proxyRequest(
        {
          hostname: host,
          port: Number(port),
          method: "GET",
          path: "http://origin.invalid/never-store-business-path",
          headers: { Host: "origin.invalid" },
        },
        (response) => {
          expect(response.statusCode).toBe(403);
          response.resume();
          response.on("end", resolve);
        },
      );
      outgoing.on("error", reject);
      outgoing.end();
    });
    await expect
      .poll(async () => {
        const value = await (
          await request.get("/api/v1/observability?window_secs=300")
        ).json();
        return value.correlations.groups.some(
          (group: { destination_host: string }) =>
            group.destination_host === "origin.invalid",
        );
      })
      .toBe(true);
    const snapshot = await (
      await request.get("/api/v1/observability?window_secs=300")
    ).json();
    expect(JSON.stringify(snapshot)).not.toContain("never-store-business-path");
    expect(
      snapshot.rule_hits.counters.some(
        (counter: { configuration_revision: number; hits: number }) =>
          counter.configuration_revision === revision.id && counter.hits === 1,
      ),
    ).toBe(true);
    await page.goto("/console/overview.html");
    const trends = page.getByRole("region", { name: "流量与失败趋势" });
    await expect(trends).toContainText("观察实例");
    await expect(
      trends.getByRole("img", { name: "下载速度 B/s" }),
    ).toBeVisible();
    await trends
      .getByRole("combobox", { name: "观测时间窗口" })
      .selectOption("300");
    await expect(page).toHaveURL(/observation_window=300/);
    await page.reload();
    await expect(
      trends.getByRole("combobox", { name: "观测时间窗口" }),
    ).toHaveValue("300");
    await page.goto("/console/connections.html");
    const correlations = page.getByRole("region", { name: "连接历史关联" });
    await expect(
      correlations.getByRole("cell").filter({ hasText: "origin.invalid:80" }),
    ).toBeVisible();
    await expect(correlations).toContainText("每分钟最多 128 组");
    await page.goto("/console/rules.html");
    await expect(
      page.getByRole("region", { name: "业务规则命中统计" }),
    ).toContainText("默认动作");
    await page.setViewportSize({ width: 390, height: 844 });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: "/tmp/hps-observation-mobile.png",
      fullPage: true,
      animations: "disabled",
    });
    expect(await page.context().cookies()).toHaveLength(0);
  } finally {
    const workspace = await (
      await request.get("/api/v1/config/workspace")
    ).json();
    if (workspace.desired_revision !== 1)
      await request.post("/api/v1/config/rollback", {
        data: { revision: 1, expected_revision: workspace.desired_revision },
      });
  }
});
