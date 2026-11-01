import { test, expect, chromium } from "@playwright/test";
import { createServer, request as httpRequest, type Server } from "node:http";
import { spawn } from "node:child_process";
import { mkdtemp, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { once } from "node:events";

async function listen(server: Server) {
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const address = server.address();
  if (!address || typeof address === "string")
    throw new Error("missing address");
  return address.port;
}

test("Chromium without proxy credentials reaches an unresolvable origin through hpsd and its supplier", async ({
  request,
}) => {
  const originHeaders: (string | undefined)[] = [];
  const origin = createServer((req, res) => {
    originHeaders.push(req.headers["proxy-authorization"]);
    res.setHeader("Content-Type", "text/html; charset=utf-8");
    res.end(
      "<!doctype html><title>Proxy path verified</title><h1>Browser → hpsd → supplier → origin</h1>",
    );
  });
  const originPort = await listen(origin);
  const paths: string[] = [];
  const supplier = createServer((req, res) => {
    if (
      req.headers["proxy-authorization"] !==
      "Basic c3VwcGxpZXI6Zml4dHVyZS1zZWNyZXQ="
    ) {
      res.writeHead(407);
      res.end();
      return;
    }
    const target = new URL(req.url ?? "");
    paths.push(target.href);
    if (target.hostname !== "origin.invalid") {
      res.writeHead(502);
      res.end();
      return;
    }
    const headers = { ...req.headers };
    delete headers["proxy-authorization"];
    delete headers["proxy-connection"];
    const outbound = httpRequest(
      {
        host: "127.0.0.1",
        port: originPort,
        path: target.pathname,
        method: req.method,
        headers,
      },
      (response) => {
        res.writeHead(response.statusCode ?? 502, response.headers);
        response.pipe(res);
      },
    );
    outbound.on("error", () => {
      res.writeHead(502);
      res.end();
    });
    req.pipe(outbound);
  });
  const supplierPort = await listen(supplier);
  const reservation = createServer();
  const managementPort = await listen(reservation);
  await new Promise<void>((done) => reservation.close(() => done()));
  const directory = await mkdtemp(join(tmpdir(), "hps-browser-proxy-"));
  const config = join(directory, "hps.toml");
  await writeFile(
    config,
    `management_listen="127.0.0.1:${managementPort}"\n[[proxy.inbounds]]\nid="browser"\nprotocol="http"\nlisten="127.0.0.1:0"\n[[proxy.upstreams]]\nid="supplier"\nprotocol="http"\nhost="127.0.0.1"\nport=${supplierPort}\n[proxy.upstreams.credentials]\nusername="supplier"\npassword="fixture-secret"\n`,
    { mode: 0o600 },
  );
  const environment = { ...process.env };
  delete environment.DISPLAY;
  delete environment.WAYLAND_DISPLAY;
  const daemon = spawn(resolve("target/debug/hpsd"), ["--ephemeral", "--config", config], {
    stdio: "ignore",
    env: environment,
  });
  const daemonExited = new Promise<void>((done) => {
    daemon.once("exit", () => done());
    daemon.once("error", () => done());
  });
  let browser: Awaited<ReturnType<typeof chromium.launch>> | undefined;
  try {
    const base = `http://127.0.0.1:${managementPort}`;
    await expect
      .poll(async () => {
        try {
          return (await request.get(`${base}/healthz`)).status();
        } catch {
          return 0;
        }
      })
      .toBe(200);
    const runtime = await (await request.get(`${base}/api/v1/inbounds`)).json();
    const address: string = runtime.inbounds[0].listen;
    browser = await chromium.launch({
      headless: true,
      proxy: { server: `http://${address}`, bypass: "<-loopback>" },
    });
    const page = await browser.newPage();
    const response = await page.goto(
      `http://origin.invalid:${originPort}/via-pool`,
    );
    expect(response?.status()).toBe(200);
    await expect(page).toHaveTitle("Proxy path verified");
    await expect(page.getByRole("heading")).toHaveText(
      "Browser → hpsd → supplier → origin",
    );
    expect(paths.some((path) => path.endsWith("/via-pool"))).toBe(true);
    expect(originHeaders.length).toBeGreaterThan(0);
    expect(originHeaders.every((value) => value === undefined)).toBe(true);
    const status = await (await request.get(`${base}/api/v1/status`)).json();
    expect(status.proxy_ready).toBe(true);
    const redacted = await (await request.get(`${base}/api/v1/config`)).text();
    expect(redacted).not.toContain("fixture-secret");
  } finally {
    await browser?.close();
    daemon.kill("SIGTERM");
    const force = setTimeout(() => daemon.kill("SIGKILL"), 5000);
    await daemonExited;
    clearTimeout(force);
    origin.closeAllConnections();
    supplier.closeAllConnections();
    await Promise.all([
      new Promise<void>((done) => origin.close(() => done())),
      new Promise<void>((done) => supplier.close(() => done())),
    ]);
    await rm(directory, { recursive: true, force: true });
  }
});
