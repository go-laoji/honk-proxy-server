#!/usr/bin/env python3
"""Actual headless CLI/Daemon tasks, streaming, restart recovery and SIGINT; no vendors."""
import json
import os
import pathlib
import selectors
import signal
import subprocess
import tempfile
import time
import urllib.request
import uuid

ROOT = pathlib.Path(__file__).resolve().parents[2]


def read_line(process, pending=b"", seconds=8):
    deadline = time.monotonic() + seconds
    with selectors.DefaultSelector() as selector:
        selector.register(process.stdout, selectors.EVENT_READ)
        while b"\n" not in pending:
            remaining = deadline - time.monotonic()
            assert remaining > 0 and selector.select(remaining), "CLI stream frame timed out"
            part = os.read(process.stdout.fileno(), 65536)
            assert part, "CLI stream ended before expected frame"
            pending += part
            assert len(pending) <= 2 * 1024 * 1024, "unbounded CLI frame"
        line, pending = pending.split(b"\n", 1)
        return json.loads(line), pending


def stop(process):
    if process.poll() is None:
        process.send_signal(signal.SIGINT)
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


def main():
    children = []
    with tempfile.TemporaryDirectory(prefix="hps-task-cli-") as directory:
        directory = pathlib.Path(directory)
        bootstrap = directory / "daemon.toml"
        bootstrap.write_text("[health]\nenabled=false\n")
        candidate = directory / "candidate.toml"
        body = '[health]\nenabled=false\n[[proxy.upstreams]]\nid="cli-task-node"\nprotocol="http"\nhost="127.0.0.1"\nport=9\n[proxy.upstreams.credentials]\nusername="fixture"\npassword="private-cli-task-marker"\n'
        candidate.write_text(body)
        logs = []

        def start(listen="127.0.0.1:0"):
            name = "task-cli-" + uuid.uuid4().hex
            log_path = directory / (name + ".log")
            output = log_path.open("w")
            logs.append(output)
            process = subprocess.Popen([str(ROOT / "target/debug/hpsd"), "--ephemeral", "--config", str(bootstrap), "--listen", listen, "--instance-name", name], cwd=ROOT, stdout=output, stderr=subprocess.STDOUT)
            children.append(process)
            endpoint = None
            for _ in range(300):
                assert process.poll() is None, "isolated daemon exited"
                for line in log_path.read_text().splitlines():
                    try:
                        fields = json.loads(line).get("fields", {})
                    except ValueError:
                        continue
                    if fields.get("message", "").startswith("management API listening"):
                        endpoint = "http://" + fields["address"]
                if endpoint:
                    break
                time.sleep(0.02)
            assert endpoint
            with urllib.request.urlopen(endpoint + "/api/v1/status", timeout=3) as response:
                status = json.load(response)
                assert status["instance_name"] == name
            return process, endpoint, status["instance_id"]

        def cli(*args, expected_code=0):
            result = subprocess.run([str(ROOT / "target/debug/hps"), "--endpoint", endpoint, "--output", "json", *map(str, args)], cwd=ROOT, capture_output=True, timeout=15)
            assert result.returncode == expected_code, (args, result.returncode, result.stderr.decode())
            return json.loads(result.stdout) if expected_code == 0 else None

        def follow(*args):
            process = subprocess.Popen([str(ROOT / "target/debug/hps"), "--endpoint", endpoint, "--output", "json", "--timeout", "1", "event", "follow", *map(str, args)], cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.PIPE, bufsize=0)
            children.append(process)
            return process

        try:
            daemon, endpoint, epoch = start()
            cli("task", "submit-config", candidate, "--expected-revision", 1, "--expected-epoch", str(uuid.uuid4()), "--request-id", "stale-submit", "--prepare", expected_code=7)
            assert cli("task", "list")["records"] == []
            accepted = cli("task", "submit-config", candidate, "--expected-revision", 1, "--request-id", "cli-prepare", "--prepare")
            task = cli("task", "wait", accepted["id"], "--expected-epoch", epoch, "--wait-secs", 5)
            assert task["state"] == "completed" and task["result_ref"] == "config:2"
            assert cli("task", "submit-config", candidate, "--expected-revision", 1, "--request-id", "cli-prepare", "--prepare")["id"] == task["id"]
            candidate.write_text(body + "\n# changed input fingerprint\n")
            cli("task", "submit-config", candidate, "--expected-revision", 1, "--request-id", "cli-prepare", "--prepare", expected_code=7)
            candidate.write_text(body)
            cli("task", "submit-config", candidate, "--expected-revision", 1, "--request-id", "cli-prepare", expected_code=7)
            page = cli("task", "list", "--kind", "configuration_prepare", "--request-id", "cli-prepare", "--limit", 1)
            assert len(page["records"]) == 1 and page["records"][0]["result"] is None
            assert cli("task", "show", task["id"], "--expected-epoch", epoch)["result"] == {"revision": 2}
            cli("task", "show", task["id"], "--expected-epoch", str(uuid.uuid4()), expected_code=7)
            cli("task", "show", 0, expected_code=2)
            cli("event", "list", "--cursor", "bad", expected_code=2)
            events = cli("event", "list", "--task-id", task["id"])
            assert any(e["code"] == "task_completed" for e in events["records"])
            assert "private-cli-task-marker" not in json.dumps(task) + json.dumps(events)

            stream = follow("--module", "configuration", "--max-frames", 2, "--reconnect-attempts", 0)
            first, pending = read_line(stream)
            assert first["kind"] == "snapshot"
            next_task = cli("task", "submit-config", candidate, "--expected-revision", 1, "--request-id", "cli-live-validate")
            stdout, stderr = stream.communicate(timeout=10)
            assert stream.returncode == 0, stderr.decode()
            frames = [first] + [json.loads(line) for line in (pending + stdout).splitlines()]
            assert len(frames) == 3 and frames[-1]["kind"] == "end" and frames[-1]["frames"] == 2
            assert any(e["task_id"] == next_task["id"] and e["code"] == "task_completed" for e in frames[1]["data"]["records"])
            assert "private-cli-task-marker" not in json.dumps(frames)

            stream = follow("--module", "configuration", "--max-frames", 3, "--reconnect-attempts", 5)
            first, pending = read_line(stream)
            assert first["kind"] == "snapshot" and first["data"]["epoch"] == epoch
            stop(daemon)
            daemon, same_endpoint, new_epoch = start(endpoint.removeprefix("http://"))
            assert same_endpoint == endpoint and new_epoch != epoch
            transport_states = []
            while True:
                frame, pending = read_line(stream, pending)
                if frame["kind"] == "transport":
                    transport_states.append(frame["data"]["state"])
                else:
                    assert frame["kind"] == "reset", frame
                    assert frame["data"]["reset_reason"] == "instance_changed"
                    assert frame["data"]["epoch"] == new_epoch and frame["data"]["records"] == []
                    break
            assert "reconnecting" in transport_states
            latest = cli("task", "submit-config", candidate, "--expected-revision", 1, "--request-id", "new-instance-validate")
            stdout, stderr = stream.communicate(timeout=10)
            assert stream.returncode == 0, stderr.decode()
            tail = [json.loads(line) for line in (pending + stdout).splitlines()]
            assert tail[0]["kind"] == "events" and tail[0]["data"]["epoch"] == new_epoch
            assert any(e["task_id"] == latest["id"] for e in tail[0]["data"]["records"])
            assert tail[-1]["kind"] == "end" and tail[-1]["frames"] == 3
            stream = follow("--reconnect-attempts", 0)
            read_line(stream)
            stream.send_signal(signal.SIGINT)
            stdout, stderr = stream.communicate(timeout=5)
            assert stream.returncode == 0, stderr.decode()
            assert json.loads(stdout.splitlines()[-1])["reason"] == "interrupted"
            print(json.dumps({"result": "passed", "headless": True, "idempotent_preparation": True, "revision": 2, "changed_input_exit": 7, "invalid_cursor_exit": 2, "real_sse": True, "restart_reset": True, "same_endpoint_new_epoch": True, "interrupt_releases_subscription": True}))
        finally:
            for process in reversed(children):
                stop(process)
            for output in logs:
                output.close()


if __name__ == "__main__":
    main()
