#!/usr/bin/env python3
"""Actual launchd/systemd acceptance. Own label, ports and state; no current proxy edits."""
import argparse
import http.server
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import tempfile
import threading
import time
import urllib.request


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def eventually(operation, timeout=25):
    end = time.monotonic() + timeout
    last = None
    while time.monotonic() < end:
        try:
            value = operation()
            if value:
                return value
        except (OSError, ValueError, AssertionError) as error:
            last = error
        time.sleep(0.1)
    raise AssertionError(f"acceptance timed out: {last}")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cli", required=True, type=Path)
    parser.add_argument("--daemon", required=True, type=Path)
    parser.add_argument("--system", action="store_true")
    options = parser.parse_args()
    cli, daemon = options.cli.resolve(), options.daemon.resolve()
    root = Path(tempfile.mkdtemp(prefix="hps-service-e2e-"))
    env = dict(os.environ, HPS_CLIENT_STATE_DIR=str(root / "client"))
    identity = "validation-" + root.name.removeprefix("hps-service-e2e-").replace("_", "a")
    management, inbound = free_port(), free_port()
    endpoint = f"http://127.0.0.1:{management}/"
    config = root / "daemon.toml"
    config.write_text(f'instance_name = "service-e2e"\nmanagement_listen = "127.0.0.1:{management}"\n[proxy.routing]\ndefault_action = {{kind="direct"}}\n[[proxy.inbounds]]\nid="http"\nprotocol="http"\nlisten="127.0.0.1:{inbound}"\n')

    def command(*args, ok=True):
        process = subprocess.run([str(cli), "--output", "json", "service", "--local", "--id", identity, *args], env=env, capture_output=True, text=True, timeout=30)
        assert (process.returncode == 0) == ok, (args, process.returncode, process.stderr)
        return json.loads(process.stdout) if ok else process.stderr

    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def status():
        with opener.open(endpoint + "api/v1/status", timeout=1) as response:
            return json.load(response)

    install = ["install", "--binary", str(daemon), "--config", str(config), "--address", endpoint]
    if options.system:
        install.append("--system")
    first = command(*install)
    unit = Path(first["manifest"]["unit"])
    original = unit.read_bytes()
    installed = True
    proof = {"platform": os.uname().sysname, "scope": "system" if options.system else "user", "state_dir": str(root), "label_id": identity}
    try:
        command("start", "--expected-endpoint", endpoint)
        ready = eventually(status)
        assert ready["management_ready"] and ready["proxy_ready"]
        initial = command("status")
        pid = initial["pid"]
        assert pid and initial["installed"]
        data = Path(initial["manifest"]["data"])
        # WAL/SHM are transient SQLite implementation files, removed after clean shutdown.
        databases = list(data.glob("*.sqlite3")) + list(data.glob("*.db"))
        assert databases, list(data.iterdir())

        class Target(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                self.send_response(200)
                self.end_headers()
                self.wfile.write(b"actual-system-service-proxy")

            def log_message(self, *args):
                pass

        with http.server.ThreadingHTTPServer(("127.0.0.1", 0), Target) as target:
            thread = threading.Thread(target=target.serve_forever, daemon=True)
            thread.start()
            with socket.create_connection(("127.0.0.1", inbound), timeout=3) as connection:
                address = f"127.0.0.1:{target.server_port}"
                connection.sendall(f"GET http://{address}/ HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n".encode())
                payload = b""
                while chunk := connection.recv(4096):
                    payload += chunk
                assert b"actual-system-service-proxy" in payload, payload
            target.shutdown()
        proof["actual_proxy_payload"] = True

        assert "target changed" in command("stop", "--expected-endpoint", "http://127.0.0.1:1/", ok=False)
        unit.write_bytes(original + b"\n# foreign change\n")
        assert "ownership conflict" in command("stop", "--expected-endpoint", endpoint, ok=False)
        assert status()["instance_id"] == ready["instance_id"]
        unit.write_bytes(original)
        proof["wrong_target_and_foreign_unit_preserved"] = True

        os.kill(pid, signal.SIGKILL)
        def new_instance():
            value = status()
            return value if value["instance_id"] != ready["instance_id"] else None
        restarted = eventually(new_instance)
        # The command process has already exited; only the native OS manager can restart it.
        proof["sigkill_os_restart"] = restarted["instance_id"] != ready["instance_id"]
        assert all(path.exists() for path in databases)
        command("stop", "--expected-endpoint", endpoint)
        eventually(lambda: command("status")["pid"] is None)
        command("uninstall", "--expected-endpoint", endpoint)
        installed = False
        assert not unit.exists() and all(path.exists() for path in databases)
        command(*install)
        installed = True
        command("start", "--expected-endpoint", endpoint)
        eventually(status)
        assert all(path.exists() for path in databases)
        proof["uninstall_and_reinstall_preserve_data"] = True
        proof["manager"] = initial["manager"]
    finally:
        if unit.exists() and unit.read_bytes() != original:
            unit.write_bytes(original)
        if installed:
            command("uninstall", "--expected-endpoint", endpoint)
    proof["passed"] = True
    print(json.dumps(proof, indent=2))


if __name__ == "__main__":
    main()
