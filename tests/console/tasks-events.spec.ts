import { test, expect, type APIRequestContext } from "@playwright/test";
import { spawn, type ChildProcess } from "node:child_process";
import { randomUUID } from "node:crypto";

test.use({ actionTimeout: 10_000, navigationTimeout: 15_000 });

type Service = {
  process: ChildProcess;
  endpoint: string;
  epoch: string;
  stop: () => Promise<void>;
};
async function start(
  request: APIRequestContext,
  listen = "127.0.0.1:0",
): Promise<Service> {
  const name = `browser-task-${randomUUID()}`;
  const process = spawn("target/debug/hpsd", [
    "--ephemeral",
    "--config",
    "tests/fixtures/console.toml",
    "--listen",
    listen,
    "--instance-name",
    name,
  ]);
  let output = "";
  process.stdout.on("data", (chunk) => {
    output += chunk.toString();
  });
  process.stderr.on("data", (chunk) => {
    output += chunk.toString();
  });
  const stop = async () => {
    if (process.exitCode !== null) return;
    const exited = new Promise<void>((resolve) =>
      process.once("exit", () => resolve()),
    );
    process.kill("SIGINT");
    const timer = setTimeout(() => process.kill("SIGKILL"), 5000);
    await exited;
    clearTimeout(timer);
  };
  try {
    let endpoint = "";
    await expect
      .poll(() => {
        for (const line of output.split("\n")) {
          try {
            const fields = JSON.parse(line).fields;
            if (fields?.message?.startsWith("management API listening"))
              endpoint = `http://${fields.address}`;
          } catch {
            /* A log may still be arriving. */
          }
        }
        return endpoint;
      })
      .not.toBe("");
    const status = await (
      await request.get(`${endpoint}/api/v1/status`)
    ).json();
    expect(status.instance_name).toBe(name);
    return { process, endpoint, epoch: status.instance_id, stop };
  } catch (error) {
    await stop();
    throw error;
  }
}
async function submit(
  request: APIRequestContext,
  service: Service,
  id: string,
) {
  const workspace = await (
    await request.get(`${service.endpoint}/api/v1/config/workspace`)
  ).json();
  const response = await request.post(`${service.endpoint}/api/v1/tasks`, {
    data: {
      expected_epoch: service.epoch,
      request_id: id,
      operation: {
        kind: "configuration_validate",
        input: {
          toml: workspace.desired_toml,
          expected_revision: workspace.desired_revision,
          note: "private-browser-task-note",
        },
      },
    },
  });
  expect(response.status()).toBe(202);
  const task = await response.json();
  await expect
    .poll(
      async () =>
        (
          await (
            await request.get(
              `${service.endpoint}/api/v1/tasks/${task.id}?expected_epoch=${service.epoch}`,
            )
          ).json()
        ).state,
    )
    .toBe("completed");
  return task;
}

test("task MPA submits recoverable work and follows epoch-bound configuration, health and event links", async ({
  page,
  request,
}) => {
  test.setTimeout(90_000);
  const service = await start(request);
  try {
    await page.goto(`${service.endpoint}/console/tasks.html`);
    await expect(page.getByLabel("事件订阅状态")).toContainText("实时");
    await page
      .getByRole("button", { name: "后台提交配置任务", exact: true })
      .click();
    await page
      .getByRole("combobox", { name: "任务动作", exact: true })
      .selectOption("prepare");
    await page
      .getByLabel("幂等请求 ID", { exact: true })
      .fill("browser-prepare-recovery");
    const toml = page.getByRole("textbox", {
      name: "后台配置 TOML",
      exact: true,
    });
    const initial = await toml.inputValue();
    await toml.fill(initial + "\n# private-browser-task-note\n");
    await page.getByRole("button", { name: "提交到后台", exact: true }).click();
    await expect(page).toHaveURL(
      /request_id=browser-prepare-recovery.*detail=/,
    );
    const selected = new URL(page.url()).searchParams.get("detail")!;
    const detail = page.getByLabel("统一任务详情", { exact: true });
    await expect(detail).toContainText("config:2");
    await expect(detail).toContainText("已完成");
    await page.getByRole("button", { name: "提交到后台", exact: true }).click();
    await expect(page).toHaveURL(new RegExp(`detail=${selected}(?:&|$)`));
    const tasks = await (
      await request.get(
        `${service.endpoint}/api/v1/tasks?request_id=browser-prepare-recovery`,
      )
    ).json();
    expect(tasks.records).toHaveLength(1);
    await toml.fill(initial + "\n# changed-input\n");
    await page.getByRole("button", { name: "提交到后台", exact: true }).click();
    await expect(page.getByRole("alert")).toContainText("冲突");
    await page.goto(
      `${service.endpoint}/console/tasks.html?request_id=browser-prepare-recovery&detail=${selected}&epoch=${service.epoch}`,
    );
    await expect(detail).toContainText("config:2");
    await detail.getByRole("link", { name: "配置版本详情" }).click();
    const revision = page.getByLabel("关联配置版本详情");
    await expect(revision).toContainText("关联版本 r2");
    await expect(revision).not.toContainText("private-browser-task-note");
    await page.goto(
      `${service.endpoint}/console/tasks.html?detail=${selected}&epoch=${service.epoch}`,
    );
    await page.getByRole("link", { name: "该任务的结构化事件" }).click();
    const events = page.getByLabel("服务事件列表");
    await expect(events).toContainText(`任务 #${selected}`);
    await expect(events).not.toContainText("private-browser-task-note");
    await events
      .getByRole("button", { name: "事件详情", exact: true })
      .first()
      .click();
    await expect(page.getByLabel("事件详情", { exact: true })).toContainText(
      "task_completed",
    );
    const healthPolicy = await (
      await request.get(`${service.endpoint}/api/v1/health`)
    ).json();
    const enabled = await request.post(
      `${service.endpoint}/api/v1/health/save`,
      {
        data: {
          configuration: { ...healthPolicy.configuration, enabled: true },
          expected_revision: 1,
        },
      },
    );
    expect(enabled.ok()).toBe(true);
    const probe = await request.post(
      `${service.endpoint}/api/v1/health/probe`,
      { data: { request_id: "browser-empty-health", node_ids: [] } },
    );
    expect(probe.status()).toBe(202);
    const health = await probe.json();
    await page.goto(
      `${service.endpoint}/console/tasks.html?source_ref=health:${health.id}&epoch=${service.epoch}`,
    );
    await expect(
      page.getByLabel("统一任务列表").getByRole("article"),
    ).toHaveCount(1);
    await page
      .getByRole("button", { name: "查看任务详情", exact: true })
      .click();
    await page.getByRole("link", { name: "健康任务详情" }).click();
    await expect(page.getByLabel("健康检测任务")).toContainText(
      `health:${health.id} · 0/0`,
    );
    await page.getByRole("link", { name: "查看统一任务与关联事件" }).click();
    await expect(
      page.getByLabel("统一任务列表").getByRole("article"),
    ).toHaveCount(1);
    await page.setViewportSize({ width: 390, height: 844 });
    await expect
      .poll(() =>
        page
          .locator("aside")
          .evaluate((el) => el.getBoundingClientRect().right),
      )
      .toBeLessThanOrEqual(0);
    await page.screenshot({
      path: "/tmp/hps-t14-tasks-mobile.png",
      fullPage: true,
      animations: "disabled",
    });
    expect(
      await page
        .locator("body")
        .evaluate((el) => el.scrollWidth <= window.innerWidth),
    ).toBe(true);
  } finally {
    await service.stop();
  }
});

test("event MPA freezes while paused, resumes without a gap and resets after same-endpoint restart", async ({
  page,
  request,
}) => {
  test.setTimeout(90_000);
  let service = await start(request);
  const endpoint = service.endpoint,
    oldEpoch = service.epoch;
  try {
    await submit(request, service, "before-pause");
    await page.goto(`${endpoint}/console/events.html?module=configuration`);
    const events = page.getByLabel("服务事件列表");
    await expect(events).toContainText("before-pause");
    await expect(page.getByLabel("事件订阅状态")).toContainText("实时");
    await page
      .getByRole("button", { name: "暂停实时订阅", exact: true })
      .click();
    await expect(page.getByLabel("事件订阅状态")).toContainText("暂停");
    await submit(request, service, "while-paused");
    await expect(events).not.toContainText("while-paused");
    await page
      .getByRole("button", { name: "恢复实时订阅", exact: true })
      .click();
    await expect(events).toContainText("while-paused");
    await events
      .getByRole("button", { name: "事件详情", exact: true })
      .first()
      .click();
    await expect(page.getByLabel("事件详情", { exact: true })).toBeVisible();
    const oldCursor = (
      await (await request.get(`${endpoint}/api/v1/events`)).json()
    ).cursor;
    await service.stop();
    service = await start(request, endpoint.replace("http://", ""));
    expect(service.epoch).not.toBe(oldEpoch);
    await expect(page.getByText("观测周期：", { exact: false })).toContainText(
      service.epoch,
    );
    await expect(events.getByRole("article")).toHaveCount(0);
    await expect(page.getByLabel("事件详情", { exact: true })).toHaveCount(0);
    await expect(
      page.getByText("服务实例已变化", { exact: false }),
    ).toBeVisible();
    await submit(request, service, "after-restart");
    await expect(events).toContainText("after-restart");
    await page.goto(
      `${endpoint}/console/events.html?cursor=${encodeURIComponent(oldCursor)}`,
    );
    await expect(events).toContainText("after-restart");
    await expect(
      page.getByText("服务实例已变化", { exact: false }),
    ).toBeVisible();
    await page.goto(`${endpoint}/console/events.html?epoch=${oldEpoch}`);
    await expect(page.getByRole("alert")).toContainText(
      "关联链接所属的服务实例已变化",
    );
    await expect(events.getByRole("article")).toHaveCount(0);
    await page
      .getByRole("button", { name: "清除实例限制并重新查询", exact: true })
      .click();
    await expect(events).toContainText("after-restart");
    await page
      .getByRole("combobox", { name: "模块", exact: true })
      .selectOption("management");
    await page
      .getByRole("button", { name: "应用事件筛选", exact: true })
      .click();
    await expect(page).toHaveURL(/module=management/);
    await expect(events).toContainText("management_operation");
    await events
      .getByRole("button", { name: "事件详情", exact: true })
      .first()
      .click();
    await expect(page.getByLabel("事件详情", { exact: true })).toContainText(
      "127.0.0.1:",
    );
    await page.setViewportSize({ width: 390, height: 844 });
    await expect
      .poll(() =>
        page
          .locator("aside")
          .evaluate((el) => el.getBoundingClientRect().right),
      )
      .toBeLessThanOrEqual(0);
    await page.screenshot({
      path: "/tmp/hps-t14-events-mobile.png",
      fullPage: true,
      animations: "disabled",
    });
    expect(
      await page
        .locator("body")
        .evaluate((el) => el.scrollWidth <= window.innerWidth),
    ).toBe(true);
  } finally {
    await service.stop();
  }
});
