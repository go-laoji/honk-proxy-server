import { test, expect } from "@playwright/test";
import http from "node:http";
import net, { type AddressInfo, type Socket } from "node:net";

type Flow = { id: string; context: { source_port: number } };
type FlowList = { records: Flow[]; filtered_total: number };

function tunnel(port: number, target: number, sockets: Set<Socket>) {
  return new Promise<Socket>((resolve, reject) => {
    const socket = net.connect(port, "127.0.0.1");
    sockets.add(socket);
    socket.on("error", reject);
    socket.setTimeout(5000, () => socket.destroy(new Error("fixture timeout")));
    socket.once("connect", () =>
      socket.write(
        `CONNECT 127.0.0.1:${target} HTTP/1.1\r\nHost: 127.0.0.1:${target}\r\n\r\n`,
      ),
    );
    let head = "";
    const read = (data: Buffer) => {
      head += data.toString();
      if (!head.includes("\r\n\r\n")) return;
      socket.off("data", read);
      socket.setTimeout(0);
      if (!head.startsWith("HTTP/1.1 200")) {
        reject(new Error(`CONNECT failed: ${head.slice(0, 100)}`));
        socket.destroy();
      } else resolve(socket);
    };
    socket.on("data", read);
  });
}

async function echo(socket: Socket, payload: string) {
  const received = new Promise<string>((resolve, reject) => {
    let data = "";
    const timer = setTimeout(() => {
      socket.off("data", read);
      reject(new Error("echo timeout"));
    }, 5000);
    const read = (chunk: Buffer) => {
      data += chunk.toString();
      if (data.length >= payload.length) {
        clearTimeout(timer);
        socket.off("data", read);
        resolve(data);
      }
    };
    socket.on("data", read);
  });
  socket.write(payload);
  expect(await received).toBe(payload);
}

test("connections MPA freezes pages and disconnect scopes while new real tunnels survive", async ({
  page,
  request,
}) => {
  test.setTimeout(90_000);
  const sockets = new Set<Socket>();
  const track = (socket: Socket) => {
    sockets.add(socket);
    socket.on("error", () => {});
  };
  const origin = net.createServer((socket) => {
    track(socket);
    socket.pipe(socket);
  });
  let upstreamConnections = 0;
  const upstream = http.createServer();
  upstream.on("connection", track);
  upstream.on("connect", (req, client, head) => {
    upstreamConnections++;
    const target = new URL(`http://${req.url}`);
    const remote = net.connect(Number(target.port), target.hostname, () => {
      client.write("HTTP/1.1 200 Connection Established\r\n\r\n");
      if (head.length) remote.write(head);
      client.pipe(remote).pipe(client);
    });
    track(remote);
    client.on("close", () => remote.destroy());
    remote.on("close", () => client.destroy());
  });
  await new Promise<void>((resolve) => origin.listen(0, "127.0.0.1", resolve));
  await new Promise<void>((resolve) =>
    upstream.listen(0, "127.0.0.1", resolve),
  );
  const targetPort = (origin.address() as AddressInfo).port;
  const upstreamPort = (upstream.address() as AddressInfo).port;
  try {
    const baseline = await (
      await request.get("/api/v1/config/revisions/1")
    ).json();
    const workspace = await (
      await request.get("/api/v1/config/workspace")
    ).json();
    const revisionResponse = await request.post("/api/v1/config/revisions", {
      data: {
        toml:
          baseline.toml.replace("upstreams = []\n", "") +
          `\n[[proxy.upstreams]]\nid="browser-observe"\nprotocol="http"\nhost="127.0.0.1"\nport=${upstreamPort}\n[proxy.upstreams.credentials]\nusername="fixture-user"\npassword="fixture-secret"\n`,
        expected_revision: workspace.desired_revision,
        note: "connection observation fixture",
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
    const start = await request.post("/api/v1/inbounds/browser-http/start");
    expect(start.ok()).toBe(true);
    const port = Number((await start.json()).listen.split(":").at(-1));
    const first = await tunnel(port, targetPort, sockets);
    const second = await tunnel(port, targetPort, sockets);
    await echo(first, "first-private-payload");
    await echo(second, "second-private-payload");
    const firstClosed = new Promise<void>((resolve) =>
      first.once("close", resolve),
    );
    const secondClosed = new Promise<void>((resolve) =>
      second.once("close", resolve),
    );
    await page.goto("/console/connections.html?node=browser-observe&limit=1");
    const list = page.getByLabel("连接列表", { exact: true });
    await expect(list.getByRole("article")).toHaveCount(1);
    await expect(
      page.getByText("匹配 2 · 当前偏移 0", { exact: true }),
    ).toBeVisible();
    await page
      .getByRole("button", { name: "暂停自动刷新", exact: true })
      .click();
    await expect(
      page.getByRole("combobox", { name: "每页", exact: true }),
    ).toHaveValue("1");
    const evictSnapshots = async () => {
      for (let n = 0; n < 4; n++) {
        expect(
          (await request.get("/api/v1/connections?node=browser-observe")).ok(),
        ).toBe(true);
      }
    };
    await evictSnapshots();
    await page.getByRole("button", { name: "下一页", exact: true }).click();
    await expect(page.getByRole("status")).toContainText("原快照已失效");
    await expect(page).not.toHaveURL(/offset=/);
    await expect(
      page.getByRole("button", { name: "刷新连接", exact: true }),
    ).toBeEnabled();
    const third = await tunnel(port, targetPort, sockets);
    await echo(third, "third-private-payload");
    const current: FlowList = await (
      await request.get("/api/v1/connections?node=browser-observe")
    ).json();
    expect(current.filtered_total).toBe(3);
    const oldIds = current.records
      .filter((r) =>
        [first.localPort, second.localPort].includes(r.context.source_port),
      )
      .map((r) => r.id);
    const thirdId = current.records.find(
      (r) => r.context.source_port === third.localPort,
    )!.id;
    expect(oldIds).toHaveLength(2);
    await page
      .getByRole("button", { name: "预览断开当前筛选", exact: true })
      .click();
    const confirm = page.getByLabel("断开确认", { exact: true });
    for (const id of oldIds) await expect(confirm).toContainText(id);
    await expect(confirm).not.toContainText(thirdId);
    await evictSnapshots();
    await confirm
      .getByRole("button", { name: "确认断开 2 条连接", exact: true })
      .click();
    await expect(page.getByRole("status")).toContainText("2 条已请求取消");
    await Promise.all([firstClosed, secondClosed]);
    await echo(third, "survives-frozen-disconnect");
    expect(upstreamConnections).toBe(3);
    await expect(
      page.getByText("匹配 1 · 当前偏移 0", { exact: true }),
    ).toBeVisible();
    await list.getByRole("button", { name: "路由详情", exact: true }).click();
    const details = page.getByLabel("连接路由详情", { exact: true });
    await expect(details).toContainText(thirdId);
    await expect(details).toContainText(`默认动作 · r${revision.id}`);
    await expect(details).toContainText("显式代理不会推断远端客户端的进程");
    await expect(page.locator("body")).not.toContainText("fixture-secret");
    await expect(page.locator("body")).not.toContainText(
      "third-private-payload",
    );
    await details.getByRole("link", { name: /browser-observe · http/ }).click();
    await expect(page).toHaveURL(/node-detail\.html\?id=browser-observe/);
    await page
      .getByRole("link", { name: "1，查看业务交换 / 隧道 →", exact: true })
      .click();
    await expect(page).toHaveURL(/connections\.html\?node=browser-observe/);
    await expect(list.getByRole("article")).toHaveCount(1);
    await page
      .getByRole("button", { name: "暂停自动刷新", exact: true })
      .click();
    await page
      .getByRole("textbox", { name: "客户端 IP / CIDR", exact: true })
      .fill("127.0.0.0/8");
    await page
      .getByRole("spinbutton", { name: "目标端口", exact: true })
      .fill(String(targetPort));
    await page.getByRole("button", { name: "应用筛选", exact: true }).click();
    await expect(page).toHaveURL(/source=127.0.0.0%2F8/);
    await page.reload();
    await expect(
      page.getByRole("textbox", { name: "客户端 IP / CIDR", exact: true }),
    ).toHaveValue("127.0.0.0/8");
    await expect(list.getByRole("article")).toHaveCount(1);
    await page.setViewportSize({ width: 390, height: 844 });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: "/tmp/hps-t13-connections-mobile.png",
      fullPage: true,
      animations: "disabled",
    });
    await list.getByRole("button", { name: "断开", exact: true }).click();
    await confirm
      .getByRole("button", { name: "取消断开", exact: true })
      .click();
    await echo(third, "cancelled-review-keeps-flow");
    const thirdClosed = new Promise<void>((resolve) =>
      third.once("close", resolve),
    );
    await list.getByRole("button", { name: "断开", exact: true }).click();
    await confirm
      .getByRole("button", { name: "确认断开 1 条连接", exact: true })
      .click();
    await thirdClosed;
    await expect(page.getByRole("status")).toContainText("1 条已请求取消");
    await expect(list.getByRole("article")).toHaveCount(0);
  } finally {
    for (const socket of sockets) socket.destroy();
    await request.post("/api/v1/inbounds/browser-http/stop");
    const workspace = await (
      await request.get("/api/v1/config/workspace")
    ).json();
    await request.post("/api/v1/config/rollback", {
      data: { revision: 1, expected_revision: workspace.desired_revision },
    });
    await Promise.all([
      new Promise<void>((resolve) => upstream.close(() => resolve())),
      new Promise<void>((resolve) => origin.close(() => resolve())),
    ]);
  }
});
