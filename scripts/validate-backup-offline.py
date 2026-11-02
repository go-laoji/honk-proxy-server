#!/usr/bin/env python3
"""Exercise real hps/hpsd backup and offline restore on an owned disposable database."""
import argparse
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import tempfile
import time
import urllib.request
import uuid

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--hps", type=Path, required=True)
    parser.add_argument("--hpsd", type=Path, required=True)
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="hps-issue25-backup-") as temporary:
        root = Path(temporary)
        data = root / "data"
        data.mkdir()
        snapshot = root / "snapshot.sqlite3"
        environment = dict(os.environ, HPS_FORBID_HOST_NETWORK_CHANGES="1", HPS_CLIENT_STATE_DIR=str(root / "client"))
        for key in ("HPS_ENDPOINT", "HPS_CA_CERT"):
            environment.pop(key, None)
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            port = reservation.getsockname()[1]
        endpoint = f"http://127.0.0.1:{port}"
        name = "hps-issue25-" + uuid.uuid4().hex
        log = (root / "daemon.log").open("w")
        process = None

        def stop():
            if process is not None and process.poll() is None:
                process.send_signal(signal.SIGINT)
                process.wait(timeout=10)

        def boot():
            owned = subprocess.Popen([
                str(args.hpsd.resolve()), "--config", str(ROOT / "tests/fixtures/console.toml"),
                "--data-dir", str(data), "--listen", f"127.0.0.1:{port}", "--instance-name", name,
            ], env=environment, stdout=log, stderr=log)
            try:
                for _ in range(100):
                    if owned.poll() is not None:
                        raise RuntimeError("owned validation daemon exited before startup")
                    try:
                        with urllib.request.urlopen(endpoint + "/api/v1/status", timeout=0.2) as response:
                            status = json.load(response)
                        if status["instance_name"] != name:
                            raise RuntimeError("validation port belongs to another service; no mutation attempted")
                        return owned
                    except OSError:
                        time.sleep(0.05)
                raise RuntimeError("owned validation daemon did not become ready")
            except BaseException:
                if owned.poll() is None:
                    owned.send_signal(signal.SIGINT)
                    owned.wait(timeout=10)
                raise

        def cli(*command, expected=0):
            result = subprocess.run([str(args.hps.resolve()), "--output", "json", *command],
                                    env=environment, text=True, capture_output=True, timeout=45)
            if result.returncode != expected:
                raise RuntimeError(f"unexpected CLI exit {result.returncode}: {result.stderr}")
            return json.loads(result.stdout) if expected == 0 else None

        try:
            process = boot()
            metadata = cli("--endpoint", endpoint, "backup", "create", str(snapshot))
            assert metadata["contains_credentials"]
            digest = metadata["sha256"]
            assert cli("backup", "inspect", str(snapshot), "--sha256", digest)["sha256"] == digest
            restore_args = ("backup", "restore", str(snapshot), "--offline", "--data-dir", str(data))
            cli(*restore_args, "--sha256", digest, expected=5)
            stop()
            restored = cli(*restore_args, "--sha256", digest)
            assert restored["restart_required"] and Path(restored["preserved_database"]).is_file()
            cli(*restore_args, "--sha256", "0" * 64, expected=5)
            process = boot()
            report = {"scope": "real_cli_and_owned_disposable_daemon", "download_digest_verified": True,
                      "running_restore_refused": True, "offline_restore_and_original_preservation": True,
                      "bad_digest_refused": True, "restart_successful": True, "snapshot_bytes": metadata["size_bytes"],
                      "network_configuration_writes": False, "user_services_touched": False}
        finally:
            stop()
            log.close()
    if args.report:
        args.report.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report))


if __name__ == "__main__":
    main()
