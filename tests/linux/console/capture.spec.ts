import { expect, test } from "@playwright/test";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
const execute = promisify(execFile);
test("actual Linux capture MPA and CLI control the same kernel owner and retain failure messages", async ({
  page,
  request,
}) => {
  const initial = await (await request.get("/api/v1/linux/capture")).json();
  expect(initial.status.platform).toBe("linux");
  expect(initial.status.ready).toBe(true);
  expect(initial.status.interfaces_ready).toBe(true);
  expect(initial.status.kernel_revision).toBe(initial.running_revision);
  const container = process.env.HPS_LINUX_UI_CONTAINER;
  if (!container)
    throw new Error(
      "HPS_LINUX_UI_CONTAINER must name the owned validation container",
    );
  async function cli(action: string) {
    const { stdout } = await execute("docker", [
      "exec",
      container!,
      "/target/aarch64/debug/hps",
      "--endpoint",
      "http://127.0.0.1:17996",
      "--output",
      "json",
      "linux",
      "capture",
      "control",
      action,
      "--expected-revision",
      String(initial.running_revision),
      "--expected-epoch",
      initial.epoch,
    ]);
    return JSON.parse(stdout);
  }
  expect((await cli("stop")).state).toBe("stopped");
  expect((await cli("start")).ready).toBe(true);
  await page.goto("/console/linux.html");
  const runtime = page.getByRole("region", { name: "透明代理运行控制" });
  await expect(runtime.getByText("运行中", { exact: true })).toBeVisible();
  await runtime
    .getByRole("button", { name: "停止透明代理", exact: true })
    .click();
  await expect(runtime.getByText("已停止", { exact: true })).toBeVisible();
  let actual = await (await request.get("/api/v1/linux/capture")).json();
  expect(actual.status.ready).toBe(false);
  expect(actual.status.program_ids).toEqual([]);
  expect((await request.get("/readyz")).status()).toBe(503);
  await runtime
    .getByRole("button", { name: "启动透明代理", exact: true })
    .click();
  await expect(runtime.getByText("运行中", { exact: true })).toBeVisible();
  actual = await (await request.get("/api/v1/linux/capture")).json();
  expect(actual.status.ready).toBe(true);
  expect(actual.status.program_ids).toHaveLength(8);
  const objectInput = runtime.getByLabel("eBPF 对象路径", { exact: true });
  await objectInput.fill("/unsaved-local-edit");
  const workspace = await (
    await request.get("/api/v1/config/workspace")
  ).json();
  const externalToml = workspace.desired_toml.replace(
    /^instance_name = .*$/m,
    'instance_name = "Linux external version fixture"',
  );
  expect(externalToml).not.toBe(workspace.desired_toml);
  const created = await request.post("/api/v1/config/revisions", {
    data: {
      toml: externalToml,
      expected_revision: initial.desired_revision,
      note: "Linux UI external version fixture",
      use_draft_credentials: false,
    },
  });
  expect(created.ok()).toBe(true);
  const external = await created.json();
  expect(
    (
      await request.post("/api/v1/config/activate", {
        data: {
          revision: external.id,
          expected_revision: initial.desired_revision,
        },
      })
    ).ok(),
  ).toBe(true);
  await runtime
    .getByRole("button", { name: "刷新运行状态", exact: true })
    .click();
  await expect(runtime).toContainText("配置已变化");
  await expect(objectInput).toHaveValue("/unsaved-local-edit");
  await expect(
    runtime.getByRole("button", { name: "保存捕获配置候选", exact: true }),
  ).toBeDisabled();
  await runtime
    .getByRole("button", { name: "载入最新捕获配置", exact: true })
    .click();
  await expect(objectInput).toHaveValue(initial.desired_configuration.object);
  const active = await (await request.get("/api/v1/linux/capture")).json();
  expect(active.running_revision).toBe(external.id);
  expect(active.status.interfaces_ready).toBe(true);
  await runtime
    .getByLabel("eBPF 对象路径", { exact: true })
    .fill("/missing/hps-validation-object");
  const failed = page.waitForResponse(
    (response) =>
      response.url().endsWith("/api/v1/linux/capture/save") &&
      response.status() === 422,
  );
  await runtime
    .getByRole("button", { name: "保存捕获配置候选", exact: true })
    .click();
  await failed;
  await expect(runtime.getByRole("alert")).toContainText("object");
  await runtime
    .getByRole("button", { name: "刷新运行状态", exact: true })
    .click();
  await expect(runtime.getByRole("alert")).toContainText("object");
  expect(
    (await (await request.get("/api/v1/linux/capture")).json())
      .desired_revision,
  ).toBe(active.desired_revision);
  await runtime
    .getByLabel("eBPF 对象路径", { exact: true })
    .fill(initial.desired_configuration.object);
  await runtime.getByLabel("持久启用透明捕获", { exact: true }).uncheck();
  const saveResponse = page.waitForResponse(
    (response) =>
      response.url().endsWith("/api/v1/linux/capture/save") &&
      response.status() === 200,
  );
  await runtime
    .getByRole("button", { name: "保存捕获配置候选", exact: true })
    .click();
  const revision = await (await saveResponse).json();
  await expect(runtime.getByRole("status")).toContainText(
    `已保存候选 r${revision.id}`,
  );
  expect(revision.id).toBeGreaterThan(active.desired_revision);
  const candidate = await (await request.get("/api/v1/linux/capture")).json();
  expect(candidate.desired_revision).toBe(active.desired_revision);
  expect(candidate.running_revision).toBe(active.running_revision);
  expect(candidate.status.ready).toBe(true);
  await page.setViewportSize({ width: 390, height: 844 });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await runtime
    .getByRole("link", { name: "审阅并激活配置", exact: true })
    .click();
  await expect(
    page.getByRole("region", { name: "关联配置版本详情" }),
  ).toContainText(`关联版本 r${revision.id}`);
  expect(
    (await (await request.get("/api/v1/linux/capture")).json()).status.ready,
  ).toBe(true);
  await page
    .getByRole("button", { name: `激活版本 ${revision.id}`, exact: true })
    .click();
  await expect(page.getByRole("status")).toContainText("需重启 Daemon");
  const pendingRestart = await (
    await request.get("/api/v1/linux/capture")
  ).json();
  expect(pendingRestart.desired_revision).toBe(revision.id);
  expect(pendingRestart.desired_configuration).toBeNull();
  expect(pendingRestart.running_revision).toBe(active.running_revision);
  expect(pendingRestart.status.ready).toBe(true);
  await page.screenshot({
    path: "/tmp/hps-t15-linux-live-console.png",
    fullPage: true,
  });
});
