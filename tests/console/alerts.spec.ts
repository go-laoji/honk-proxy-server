import { test, expect } from "@playwright/test";
test("actual unavailable exit and recovery produce bounded notices and alert settings use reviewed revisions", async ({
  page,
  request,
}) => {
  const original = await (await request.get("/api/v1/config/workspace")).json();
  const effective = await (await request.get("/api/v1/config")).json();
  try {
    const saved = await (
      await request.post("/api/v1/config/revisions", {
        data: {
          toml: `management_listen="${effective.management_listen}"\n[health]\nenabled=false\n[alerts]\ncooldown_secs=5\n[[proxy.inbounds]]\nid="alert-browser"\nprotocol="http"\nlisten="127.0.0.1:0"\n`,
          expected_revision: original.desired_revision,
          note: "actual alert state test",
        },
      })
    ).json();
    expect(
      (
        await request.post("/api/v1/config/activate", {
          data: {
            revision: saved.id,
            expected_revision: original.desired_revision,
          },
        })
      ).ok(),
    ).toBe(true);
    await expect
      .poll(
        async () => {
          const f = await (
            await request.get("/api/v1/observability/alerts?limit=100")
          ).json();
          return f.notices.some(
            (n: { code: string; state: string }) =>
              n.code === "no_eligible_exit" && n.state === "problem",
          );
        },
        { timeout: 12000 },
      )
      .toBe(true);
    await page.goto("/console/events.html");
    const panel = page.getByRole("region", { name: "状态告警" });
    await expect(panel).toContainText("当前没有满足全局准入条件");
    expect(
      (await request.post("/api/v1/inbounds/alert-browser/stop")).ok(),
    ).toBe(true);
    await expect
      .poll(
        async () => {
          const f = await (
            await request.get("/api/v1/observability/alerts?limit=100")
          ).json();
          return f.notices.some(
            (n: { key: string; state: string }) =>
              n.key === "no_exit" && n.state === "recovered",
          );
        },
        { timeout: 12000 },
      )
      .toBe(true);
    await panel.getByRole("spinbutton", { name: "失败率阈值 %" }).fill("65");
    await panel.getByRole("button", { name: "保存告警设置候选" }).click();
    await expect(
      panel.getByRole("button", { name: "应用告警设置" }),
    ).toBeVisible();
    expect(
      (await (await request.get("/api/v1/observability/alerts")).json())
        .settings.failure_percent,
    ).toBe(50);
    await panel.getByRole("button", { name: "应用告警设置" }).click();
    await expect
      .poll(
        async () =>
          (await (await request.get("/api/v1/observability/alerts")).json())
            .settings.failure_percent,
      )
      .toBe(65);
    const feed = await (
      await request.get("/api/v1/observability/alerts")
    ).json();
    expect(
      (
        await request.get(
          `/api/v1/observability/alerts?after=${feed.latest_sequence}&limit=1`,
        )
      ).ok(),
    ).toBe(true);
    expect(
      (
        await (
          await request.get(
            `/api/v1/observability/alerts?after=${feed.latest_sequence}&limit=1`,
          )
        ).json()
      ).notices,
    ).toEqual([]);
    expect(
      (await request.get("/api/v1/observability/alerts?limit=101")).status(),
    ).toBe(422);
    expect(
      (
        await request.post("/api/v1/observability/alerts/save", {
          data: { expected_revision: 1, settings: feed.settings },
        })
      ).status(),
    ).toBe(409);
  } finally {
    const current = await (
      await request.get("/api/v1/config/workspace")
    ).json();
    const reset = await (
      await request.post("/api/v1/config/revisions", {
        data: {
          toml: original.desired_toml,
          expected_revision: current.desired_revision,
          note: "restore alert fixture",
        },
      })
    ).json();
    expect(
      (
        await request.post("/api/v1/config/activate", {
          data: {
            revision: reset.id,
            expected_revision: current.desired_revision,
          },
        })
      ).ok(),
    ).toBe(true);
  }
});
