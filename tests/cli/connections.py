#!/usr/bin/env python3
"""Exercise the actual CLI parser/client against an owned headless daemon and TCP flows."""
import json
import pathlib
import signal
import socket
import socketserver
import subprocess
import tempfile
import threading
import time
import urllib.request
import uuid

ROOT = pathlib.Path(__file__).resolve().parents[2]


class Echo(socketserver.BaseRequestHandler):
    def handle(self):
        body = bytearray()
        try:
            while chunk := self.request.recv(4096):
                body.extend(chunk)
            self.request.sendall(body)
        except OSError:
            pass


class Origin(socketserver.ThreadingTCPServer):
    daemon_threads = True


def main():
    with tempfile.TemporaryDirectory(prefix="hps-connection-cli-") as directory, Origin(("127.0.0.1", 0), Echo) as origin:
        thread = threading.Thread(target=origin.serve_forever, daemon=True)
        thread.start()
        directory = pathlib.Path(directory)
        config = directory / "daemon.toml"
        config.write_text('[health]\nenabled=false\n[[proxy.inbounds]]\nid="cli-http"\nprotocol="http"\nlisten="127.0.0.1:0"\nenabled=true\n[proxy.routing]\ndefault_action={kind="direct"}\n')
        name = "connection-cli-" + uuid.uuid4().hex
        sockets = []
        with (directory / "daemon.log").open("w") as output:
            daemon = subprocess.Popen([str(ROOT / "target/debug/hpsd"), "--ephemeral", "--config", str(config), "--listen", "127.0.0.1:0", "--instance-name", name], cwd=ROOT, stdout=output, stderr=subprocess.STDOUT)
            try:
                endpoint = None
                for _ in range(200):
                    assert daemon.poll() is None, "isolated daemon exited"
                    for line in (directory / "daemon.log").read_text().splitlines():
                        try:
                            fields = json.loads(line).get("fields", {})
                        except ValueError:
                            continue
                        if fields.get("message", "").startswith("management API listening"):
                            endpoint = "http://" + fields["address"]
                    if endpoint:
                        break
                    time.sleep(0.025)
                assert endpoint
                with urllib.request.urlopen(endpoint + "/api/v1/status") as response:
                    assert json.load(response)["instance_name"] == name

                def cli(*args, expected_code=0):
                    result = subprocess.run([str(ROOT / "target/debug/hps"), "--endpoint", endpoint, "--output", "json", "connection", *map(str, args)], cwd=ROOT, text=True, capture_output=True, timeout=15)
                    assert result.returncode == expected_code, (args, result.returncode, result.stderr)
                    return json.loads(result.stdout) if expected_code == 0 else None

                summary = cli("summary")
                inlet_port = int(summary["proxy"]["inbounds"][0]["listen"].rsplit(":", 1)[1])

                def tunnel():
                    peer = socket.create_connection(("127.0.0.1", inlet_port), timeout=3)
                    sockets.append(peer)
                    target = "%s:%s" % origin.server_address
                    peer.sendall(f"CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n".encode())
                    head = bytearray()
                    while not head.endswith(b"\r\n\r\n"):
                        part = peer.recv(1)
                        assert part, "proxy closed before CONNECT response"
                        head.extend(part)
                    assert head.startswith(b"HTTP/1.1 200")
                    return peer

                first, second = tunnel(), tunnel()
                page = cli("list", "--limit", 1, "--sort", "oldest")
                assert page["filtered_total"] == 2 and page["next_offset"] == 1
                epoch = page["epoch"]
                third = tunnel()
                next_page = cli("list", "--snapshot", page["snapshot"], "--expected-epoch", epoch, "--offset", 1, "--limit", 1, "--sort", "oldest")
                assert next_page["filtered_total"] == 2
                preview = cli("disconnect-preview", "--snapshot", page["snapshot"], "--expected-epoch", epoch)
                assert len(preview["targets"]) == 2
                result = cli("disconnect-confirm", "--token", preview["token"], "--expected-epoch", epoch, "--confirm")
                assert result["requested"] == 2
                for peer in (first, second):
                    assert peer.recv(1) == b""
                marker = b"cli-private-body-marker"
                third.sendall(marker)
                third.shutdown(socket.SHUT_WR)
                received = bytearray()
                while chunk := third.recv(4096):
                    received.extend(chunk)
                assert received == marker
                fourth = tunnel()
                page = cli("list")
                connection_id = page["records"][0]["id"]
                cli("disconnect", connection_id, "--expected-epoch", epoch, expected_code=2)
                assert cli("show", connection_id)["state"] == "open"
                cli("disconnect", connection_id, "--expected-epoch", epoch, "--confirm")
                assert fourth.recv(1) == b""
                cli("disconnect", connection_id, "--expected-epoch", str(uuid.uuid4()), "--confirm", expected_code=7)
                summary = cli("summary")
                assert summary["statistics"]["active"] == 0
                assert summary["statistics"]["disconnected"] == 3
                assert summary["statistics"]["bytes_up"] == len(marker)
                assert summary["statistics"]["bytes_down"] == len(marker)
                records = cli("list", "--state", "all")
                assert "cli-private-body-marker" not in json.dumps(records)
                print(json.dumps({"result": "passed", "flows_started": 4, "disconnected": 3, "bulk_scope_frozen": True, "bytes_each_direction": len(marker), "stale_epoch_exit": 7, "confirmation_required_exit": 2, "headless": True}))
            finally:
                for peer in sockets:
                    peer.close()
                if daemon.poll() is None:
                    daemon.send_signal(signal.SIGINT)
                    try:
                        daemon.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        daemon.kill()
                        daemon.wait()
                origin.shutdown()


if __name__ == "__main__":
    main()
