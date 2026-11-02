#!/usr/bin/env python3
"""Installed-binary port conflict and connection-budget recovery on owned sockets."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("release_fixture", Path(__file__).with_name("installed.py"))
fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixture)
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--bin-dir", type=Path, required=True)
parser.add_argument("--report", type=Path, required=True)
args = parser.parse_args()
binary = (args.bin_dir / "hpsd").resolve()
with tempfile.TemporaryDirectory(prefix="hps-release-faults-") as directory:
    directory = Path(directory)
    config = directory / "hps.toml"
    config.write_text('management_listen="127.0.0.1:0"\n[health]\nenabled=false\n[proxy.limits]\nmax_connections=4\n[[proxy.inbounds]]\nid="budget"\nprotocol="http"\nlisten="127.0.0.1:0"\nmax_connections=4\n')
    daemon = fixture.Daemon(binary, config, directory / "data", directory)
    held = []
    try:
        inlet = daemon.api("/api/v1/inbounds")["inbounds"][0]["listen"]
        host, port = inlet.rsplit(":", 1)
        for _ in range(4):
            held.append(socket.create_connection((host, int(port)), timeout=5))
        for _ in range(200):
            status = daemon.api("/api/v1/inbounds")
            if status["active_connections"] == 4:
                break
            time.sleep(0.01)
        fixture.require(status["active_connections"] == 4, "owned sockets did not hold the advertised budget")
        with socket.create_connection((host, int(port)), timeout=5) as extra:
            extra.settimeout(5)
            try:
                closed = not extra.recv(1)
            except ConnectionResetError:
                closed = True
            fixture.require(closed, "excess proxy connection was admitted")
        rejected = daemon.api("/api/v1/inbounds")["inbounds"][0]["rejected_global_limit"]
        fixture.require(rejected >= 1, "limit rejection counter was not real")
        for connection in held:
            connection.close()
        held.clear()
        for _ in range(200):
            status = daemon.api("/api/v1/inbounds")
            if status["active_connections"] == 0:
                break
            time.sleep(0.01)
        fixture.require(status["active_connections"] == 0, "budget was not released")
        fixture.require(daemon.api("/api/v1/status")["management_ready"], "proxy overload broke management")
    finally:
        for connection in held:
            connection.close()
        daemon.stop()
    with socket.socket() as occupied:
        occupied.bind(("127.0.0.1", 0))
        occupied.listen()
        conflict = directory / "conflict.toml"
        conflict.write_text(config.read_text().replace('management_listen="127.0.0.1:0"', f'management_listen="127.0.0.1:{occupied.getsockname()[1]}"'))
        failure = subprocess.run([str(binary), "--config", str(conflict), "--data-dir", str(directory / "conflict-data")], capture_output=True, env=dict(os.environ, HPS_FORBID_HOST_NETWORK_CHANGES="1"), timeout=15)
        fixture.require(failure.returncode != 0, "occupied management port was not rejected")
    # The failed process must release the DB owner, inbounds and runtime tasks.
    recovered = fixture.Daemon(binary, config, directory / "conflict-data", directory)
    try:
        fixture.require(recovered.api("/api/v1/status")["management_ready"], "failed startup left an unrecoverable owner")
    finally:
        recovered.stop()
result = {"passed": True, "daemon_sha256": fixture.checksum(binary), "actual_admission_limit": 4, "excess_connection_rejected": True, "rejection_counter": rejected, "management_survives_proxy_overload": True, "permits_released": True, "occupied_management_port_refused": True, "restart_after_startup_failure": True, "host_network_configuration_written": False}
args.report.parent.mkdir(parents=True, exist_ok=True)
args.report.write_text(json.dumps(result, indent=2) + "\n")
print(json.dumps(result))
