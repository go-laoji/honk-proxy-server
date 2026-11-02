#!/usr/bin/env python3
"""Actual headless process contract; isolated ephemeral daemon, no vendor or desktop needed."""
import json
import pathlib
import signal
import subprocess
import tempfile
import time
import urllib.request
import uuid

ROOT = pathlib.Path(__file__).resolve().parents[2]


def main():
    with tempfile.TemporaryDirectory(prefix="hps-node-cli-") as directory:
        directory = pathlib.Path(directory)
        name = "node-cli-" + uuid.uuid4().hex
        log = directory / "daemon.log"
        with log.open("w") as output:
            daemon = subprocess.Popen(
                [str(ROOT / "target/debug/hpsd"), "--ephemeral", "--config",
                 str(ROOT / "tests/fixtures/console.toml"), "--listen", "127.0.0.1:0",
                 "--instance-name", name], stdout=output, stderr=subprocess.STDOUT, cwd=ROOT,
            )
            try:
                endpoint = None
                for _ in range(200):
                    assert daemon.poll() is None, "isolated daemon exited"
                    for line in log.read_text().splitlines():
                        try:
                            fields = json.loads(line).get("fields", {})
                        except ValueError:
                            continue
                        if fields.get("message", "").startswith("management API listening"):
                            endpoint = "http://" + fields["address"]
                    if endpoint:
                        break
                    time.sleep(0.025)
                assert endpoint, "isolated daemon did not become ready"
                with urllib.request.urlopen(endpoint + "/api/v1/status") as response:
                    assert json.load(response)["instance_name"] == name

                def cli(*args, expected_code=0):
                    result = subprocess.run(
                        [str(ROOT / "target/debug/hps"), "--endpoint", endpoint,
                         "--output", "json", "node", *map(str, args)],
                        text=True, capture_output=True, timeout=15, cwd=ROOT,
                    )
                    assert result.returncode == expected_code, (args, result.returncode, result.stderr)
                    return json.loads(result.stdout) if expected_code == 0 else None

                assert cli("list")["summary"]["inventory"] == 0
                document = directory / "nodes.txt"
                document.write_text("http://test-user:synthetic-password@127.0.0.1:9\nsocks5://[::1]:9\n")
                preview = cli("import-preview", document, "--expected-revision", 1)
                assert preview["applicable"] and preview["valid_count"] == 2
                assert "synthetic-password" not in json.dumps(preview)
                applied = cli("import-apply", document, "--expected-revision", 1,
                              "--checksum", preview["checksum"])
                revision = applied["running_revision"]
                assert revision == 2
                inventory = cli("list", "--limit", 1)
                assert inventory["filtered_total"] == 2 and inventory["next_offset"] == 1
                node = inventory["nodes"][0]
                node_id = node["id"]
                assert cli("show", node_id)["nodes"][0]["id"] == node_id
                for action, version in [("disable", 0), ("drain", 1), ("delete", 2), ("enable", 3)]:
                    result = cli(action, node_id, "--expected-revision", revision,
                                 "--expected-control-version", version)
                    assert result["control"]["version"] == version + 1
                    assert result["pool_refreshed"]
                cli("disable", node_id, "--expected-revision", revision,
                    "--expected-control-version", 0, expected_code=7)
                cli("import-apply", document, "--expected-revision", 1,
                    "--checksum", preview["checksum"], expected_code=7)
                final = cli("show", node_id)
                assert final["nodes"][0]["operation"]["state"] == "enabled"
                assert "synthetic-password" not in json.dumps(final)
                print(json.dumps({"result": "passed", "configuration_revision": revision,
                                  "operational_version": 4, "inventory": 2,
                                  "stale_exit_code": 7, "ephemeral_instance": True}))
            finally:
                if daemon.poll() is None:
                    daemon.send_signal(signal.SIGINT)
                    try:
                        daemon.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        daemon.kill()
                        daemon.wait()


if __name__ == "__main__":
    main()
