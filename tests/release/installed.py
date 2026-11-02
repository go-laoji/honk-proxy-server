#!/usr/bin/env python3
"""Exercise installed binaries on private ports/data; never changes OS networking."""
import argparse
import concurrent.futures
import hashlib
import http.server
import json
import os
from pathlib import Path
import select
import shutil
import signal
import socket
import socketserver
import sqlite3
import statistics
import subprocess
import tempfile
import threading
import time
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
PAYLOAD = b"hps-owned-release-fixture\n" * 512


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def checksum(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


class Origin(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.send_header("Content-Length", str(len(PAYLOAD)))
        self.end_headers()
        self.wfile.write(PAYLOAD)

    def log_message(self, *args):
        pass


class OriginServer(http.server.ThreadingHTTPServer):
    request_queue_size = 256
    daemon_threads = True


class Supplier(socketserver.BaseRequestHandler):
    def handle(self):
        self.request.settimeout(10)
        header = bytearray()
        while not header.endswith(b"\r\n\r\n") and len(header) < 65536:
            block = self.request.recv(1)
            if not block:
                return
            header.extend(block)
        method, target, _ = bytes(header).split(b"\r\n", 1)[0].decode().split(" ")
        if method == "CONNECT":
            host, port = target.rsplit(":", 1)
            require(host in ("127.0.0.1", "localhost"), "fixture forbids external destinations")
        else:
            from urllib.parse import urlsplit
            url = urlsplit(target)
            require(url.hostname == "origin.invalid", "fixture expected supplier-side DNS")
            host, port = "127.0.0.1", url.port
        with socket.create_connection((host, int(port)), timeout=10) as outbound:
            if method == "CONNECT":
                self.request.sendall(b"HTTP/1.1 200 Connection established\r\n\r\n")
            else:
                outbound.sendall(header)
            readers = [self.request, outbound]
            while readers:
                ready, _, _ = select.select(readers, [], [], 10)
                if not ready:
                    return
                for source in ready:
                    destination = outbound if source is self.request else self.request
                    block = source.recv(65536)
                    if not block:
                        readers.remove(source)
                        destination.shutdown(socket.SHUT_WR)
                    else:
                        destination.sendall(block)


class Fixture(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    request_queue_size = 256
    daemon_threads = True


class Daemon:
    def __init__(self, binary, config, data, work):
        self.log = work / ("daemon-" + str(time.time_ns()) + ".log")
        environment = dict(os.environ, HPS_FORBID_HOST_NETWORK_CHANGES="1")
        environment.pop("DISPLAY", None)
        environment.pop("WAYLAND_DISPLAY", None)
        with self.log.open("w") as output:
            self.process = subprocess.Popen([str(binary), "--config", str(config), "--data-dir", str(data)], env=environment, stdout=output, stderr=subprocess.STDOUT)
        self.endpoint = None
        for _ in range(400):
            if self.process.poll() is not None:
                raise RuntimeError("owned daemon exited: " + self.log.read_text()[-4000:])
            for line in self.log.read_text().splitlines():
                try:
                    fields = json.loads(line).get("fields", {})
                    if fields.get("message", "").startswith("management API listening"):
                        self.endpoint = "http://" + fields["address"]
                except (ValueError, KeyError):
                    pass
            if self.endpoint:
                return
            time.sleep(0.025)
        self.stop()
        raise RuntimeError("owned daemon startup timed out")

    def api(self, path, data=None):
        request = urllib.request.Request(self.endpoint + path, data=None if data is None else json.dumps(data).encode(), headers={"Content-Type": "application/json"})
        # Do not inherit a developer's environment proxy for loopback checks.
        with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(request, timeout=15) as response:
            return json.load(response)

    def stop(self):
        if self.process.poll() is None:
            self.process.send_signal(signal.SIGINT)
            try:
                self.process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()


def receive(sock):
    response = bytearray()
    while True:
        block = sock.recv(65536)
        if not block:
            break
        response.extend(block)
    require(bytes(response).endswith(PAYLOAD) and b"200 OK\r\n" in response, "real proxy payload differs: " + repr(bytes(response[:256])))
    return len(response)


def exchange(address, origin_port, kind="http"):
    host, port = address.rsplit(":", 1)
    started = time.monotonic()
    with socket.create_connection((host, int(port)), timeout=15) as sock:
        sock.settimeout(15)
        if kind == "socks5":
            sock.sendall(b"\x05\x01\x00")
            require(sock.recv(2) == b"\x05\x00", "SOCKS method rejected")
            sock.sendall(b"\x05\x01\x00\x01\x7f\x00\x00\x01" + origin_port.to_bytes(2, "big"))
            reply = sock.recv(4)
            require(reply[:2] == b"\x05\x00", "SOCKS CONNECT rejected")
            remaining = 6 if reply[3] == 1 else 18
            while remaining:
                remaining -= len(sock.recv(remaining))
        elif kind == "connect":
            sock.sendall(f"CONNECT 127.0.0.1:{origin_port} HTTP/1.1\r\nHost: 127.0.0.1:{origin_port}\r\n\r\n".encode())
            header = bytearray()
            while not header.endswith(b"\r\n\r\n"):
                block = sock.recv(1)
                require(block, "CONNECT ended early")
                header.extend(block)
            require(b" 200 " in header, "HTTP CONNECT rejected")
        target = f"http://origin.invalid:{origin_port}/release" if kind == "http" else "/release"
        sock.sendall(f"GET {target} HTTP/1.1\r\nHost: origin.invalid\r\nConnection: close\r\n\r\n".encode())
        size = receive(sock)
    return (time.monotonic() - started) * 1000, size


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, required=True)
    parser.add_argument("--previous-daemon", type=Path)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    binary = (args.bin_dir / "hpsd").resolve()
    previous = args.previous_daemon.resolve() if args.previous_daemon else binary
    result = {"scope": "owned_loopback_installed_runtime", "daemon_sha256": checksum(binary), "previous_daemon_sha256": checksum(previous), "separate_previous_binary": previous != binary, "host_network_configuration_written": False}
    with tempfile.TemporaryDirectory(prefix="hps-release-runtime-") as directory, OriginServer(("127.0.0.1", 0), Origin) as origin, Fixture(("127.0.0.1", 0), Supplier) as supplier:
        work = Path(directory)
        for server in (origin, supplier):
            threading.Thread(target=server.serve_forever, daemon=True).start()
        config = work / "hps.toml"
        config.write_text('management_listen="127.0.0.1:0"\n[health]\nenabled=false\n[[proxy.inbounds]]\nid="http"\nprotocol="http"\nlisten="127.0.0.1:0"\n[[proxy.inbounds]]\nid="socks"\nprotocol="socks5"\nlisten="127.0.0.1:0"\n[[proxy.upstreams]]\nid="supplier"\nprotocol="http"\nhost="127.0.0.1"\nport=' + str(supplier.server_address[1]) + '\n')
        data = work / "data"
        daemon = Daemon(previous, config, data, work)
        try:
            exported = daemon.api("/api/v1/config/export")["toml"]
            revision = daemon.api("/api/v1/config/revisions", {"toml": exported.replace('instance_name = "honk-proxy-server"', 'instance_name = "release-persisted"'), "expected_revision": 1, "note": "upgrade fixture"})["id"]
            daemon.api("/api/v1/config/activate", {"revision": revision, "expected_revision": 1})
            require(daemon.api("/api/v1/status")["instance_name"] == "release-persisted", "initial write failed")
        finally:
            daemon.stop()
        # A real stop is required before copying the complete SQLite directory.
        snapshot = work / "stopped-snapshot"
        shutil.copytree(data, snapshot)
        daemon = Daemon(binary, config, data, work)
        try:
            require(daemon.api("/api/v1/status")["instance_name"] == "release-persisted", "upgrade lost persisted configuration")
            result["upgrade_persisted_revision"] = revision
            pages = json.loads((ROOT / "web/console/pages.manifest.json").read_text())
            opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
            for page in pages:
                with opener.open(daemon.endpoint + "/console/" + page["file"], timeout=10) as response:
                    require(response.status == 200 and b"<html" in response.read(), "MPA missing")
            result["actual_html_pages"] = len(pages)
            cli = args.bin_dir.resolve() / "hps"
            cli_status = subprocess.run([str(cli), "--endpoint", daemon.endpoint, "--output", "json", "status"], capture_output=True, text=True, timeout=15, check=True)
            require(json.loads(cli_status.stdout)["instance_name"] == "release-persisted", "installed CLI sees a different service")
            result["installed_cli_without_credentials"] = True
            conflict = subprocess.run([str(binary), "--config", str(config), "--data-dir", str(data)], capture_output=True, timeout=15)
            require(conflict.returncode != 0 and b"AlreadyRunning" in conflict.stderr, "duplicate owner was not rejected")
            result["duplicate_instance_rejected"] = True
            addresses = {v["protocol"]: v["listen"] for v in daemon.api("/api/v1/inbounds")["inbounds"]}
            for kind in ("http", "connect", "socks5"):
                exchange(addresses["socks5" if kind == "socks5" else "http"], origin.server_address[1], kind)
            started = time.monotonic()
            with concurrent.futures.ThreadPoolExecutor(max_workers=16) as pool:
                samples = list(pool.map(lambda _: exchange(addresses["http"], origin.server_address[1]), range(256)))
            elapsed = time.monotonic() - started
            latency = sorted(value[0] for value in samples)
            result["bounded_load"] = {"requests": len(samples), "workers": 16, "failures": 0, "seconds": round(elapsed, 4), "requests_per_second": round(len(samples) / elapsed, 2), "response_bytes": sum(v[1] for v in samples), "latency_ms_p50": round(statistics.median(latency), 2), "latency_ms_p95": round(latency[int(len(latency) * 0.95)], 2), "environment": "local owned fixture, not public network capacity"}
            for _ in range(200):
                active = daemon.api("/api/v1/inbounds")["active_connections"]
                if active == 0:
                    break
                time.sleep(0.025)
            require(active == 0, "load connections leaked")
            result["active_connections_after_load"] = active
        finally:
            daemon.stop()
        # Newer schema must be rejected, never coerced. Restore a stopped snapshot.
        with sqlite3.connect(data / "hps.sqlite3") as connection:
            connection.execute("INSERT INTO schema_migrations VALUES(999,0)")
        rejected = subprocess.run([str(binary), "--config", str(config), "--data-dir", str(data)], capture_output=True, timeout=15)
        require(rejected.returncode != 0 and b"SchemaTooNew" in rejected.stderr, "newer schema was not refused")
        result["newer_schema_refused"] = True
        shutil.rmtree(data)
        shutil.copytree(snapshot, data)
        daemon = Daemon(previous, config, data, work)
        try:
            require(daemon.api("/api/v1/status")["instance_name"] == "release-persisted", "rollback lost configuration")
            result["stopped_snapshot_binary_rollback"] = True
            require(daemon.api("/api/v1/tasks")["records"], "persisted task history missing after rollback")
            result["task_history_survives_upgrade_and_rollback"] = True
        finally:
            daemon.stop()
            origin.shutdown()
            supplier.shutdown()
    result["passed"] = True
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result))


if __name__ == "__main__":
    main()
