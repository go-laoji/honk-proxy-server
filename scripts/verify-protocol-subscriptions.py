#!/usr/bin/env python3
"""Own a loopback daemon and exercise the independent CLI subscription contract.
No OS network settings, paid provider, or user service is touched.
"""
import argparse
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import urllib.request

parser = argparse.ArgumentParser()
parser.add_argument("--bin-dir", required=True, type=Path)
args = parser.parse_args()
bins = args.bin_dir.resolve()
root = Path(__file__).resolve().parents[1]
env = dict(os.environ, HPS_FORBID_HOST_NETWORK_CHANGES="1")
checks = []
with socket.socket() as reservation:
    reservation.bind(("127.0.0.1", 0))
    port = reservation.getsockname()[1]
endpoint = f"http://127.0.0.1:{port}"
with tempfile.TemporaryDirectory(prefix="hps-t19-cli-") as raw:
    directory = Path(raw)
    with (directory / "daemon.log").open("w+") as log:
        daemon = subprocess.Popen([str(bins / "hpsd"), "--ephemeral", "--config", str(root / "tests/fixtures/console.toml"), "--listen", f"127.0.0.1:{port}"], env=env, stdout=log, stderr=log)
        try:
            for _ in range(200):
                if daemon.poll() is not None:
                    raise RuntimeError("owned daemon exited before readiness")
                try:
                    with urllib.request.urlopen(endpoint + "/healthz", timeout=0.2) as response:
                        if response.status == 200:
                            break
                except OSError:
                    time.sleep(0.05)
            else:
                raise RuntimeError("owned daemon readiness timed out")
            def cli(*command, success=True):
                result = subprocess.run([str(bins / "hps"), "--endpoint", endpoint, "--output", "json", *command], env=env, capture_output=True, text=True, timeout=10)
                assert "fixture-secret" not in result.stdout
                assert "fixture-password" not in result.stdout
                if success:
                    assert result.returncode == 0, result.stderr
                    return json.loads(result.stdout)
                assert result.returncode != 0
            initial = cli("subscription", "list")
            assert initial["subscriptions"] == []
            checks.append("independent CLI lists empty subscriptions")
            definition = directory / "source.toml"
            definition.write_text('id="cli-fixture"\nname="CLI fixture"\nurl="https://example.test/sub?token=fixture-secret"\nformat="uri"\n')
            saved = cli("subscription", "save", str(definition), "--expected-revision", str(initial["running_revision"]))
            assert saved["state"] == "applied"
            checks.append("definition save uses persisted revision CAS")
            document = directory / "subscription.txt"
            document.write_text("socks5://user:fixture-password@127.0.0.1:1080#fixture\n")
            refreshed = cli("subscription", "refresh", "cli-fixture", "--expected-revision", str(saved["running_revision"]), "--document", str(document))
            view = cli("subscription", "list")
            node = view["subscriptions"][0]["snapshot"]["nodes"][0]
            assert node["id"].startswith("sub:cli-fixture:") and node["expires_at_unix"] is None
            checks.append("document import publishes namespaced nodes without fabricated expiry")
            assert view["subscriptions"][0]["url"] == "[REDACTED]" and node["credentials"]["password"] == "[REDACTED]"
            checks.append("URL and protocol credentials are redacted")
            cli("subscription", "refresh", "cli-fixture", "--expected-revision", str(saved["running_revision"]), "--document", str(document), success=False)
            checks.append("stale refresh is rejected")
            assert cli("capabilities")["socks5_udp"]["available"]
            checks.append("UDP ASSOCIATE capability is reported separately")
            cli("subscription", "delete", "cli-fixture", "--expected-revision", str(refreshed["running_revision"]))
            assert cli("subscription", "list")["subscriptions"] == []
            checks.append("subscription deletion removes its active source")
        finally:
            daemon.terminate()
            try:
                daemon.wait(timeout=10)
            except subprocess.TimeoutExpired:
                daemon.kill()
                daemon.wait(timeout=5)
print(json.dumps({"status": "passed", "checks": checks, "host_network_changes": False}, indent=2))
