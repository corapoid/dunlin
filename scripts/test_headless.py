#!/usr/bin/env python3
"""Deterministic CLI/ACP acceptance; build dunlin-cli and mock_acp_agent first."""
import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import pty
import select
import shlex
import signal
import subprocess
import tempfile
import threading
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, default=Path("target/debug"))
    args = parser.parse_args()
    cli = (args.bin_dir / "dunlin-cli").resolve()
    agent = (args.bin_dir / "mock_acp_agent").resolve()
    for binary in [cli, agent]:
        if not binary.is_file():
            raise SystemExit(f"Missing fixture binary: {binary}; build CLI and mock_acp_agent first")
    with tempfile.TemporaryDirectory(prefix="dunlin-headless-") as directory:
        class RelayFixture(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def reply(self, status, value):
                data = json.dumps(value).encode()
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            def do_POST(self):
                request = json.loads(self.rfile.read(int(self.headers.get("Content-Length", "0"))))
                if self.path != "/v1/pair" or request.get("code") != "ABC123":
                    return self.reply(401, {"error": "invalid code"})
                self.reply(200, {"token": "fixture-bearer-secret", "user": {"id": "u", "name": "admin", "role": "admin"}, "device_id": "cli-device"})

            def do_GET(self):
                if self.headers.get("Authorization") != "Bearer fixture-bearer-secret":
                    return self.reply(401, {"error": "fixture-bearer-secret must not leak"})
                if self.path == "/v1/projects":
                    return self.reply(200, [{"id": "p", "name": "Fixture", "path": "/srv/project"}])
                if self.path == "/v1/sessions?project=p":
                    return self.reply(200, [{"id": "s", "title": "Fixture session", "updated_at": 42, "last_device_id": None, "active_device_id": None}])
                self.reply(404, {"error": "unknown route"})

        server = ThreadingHTTPServer(("127.0.0.1", 0), RelayFixture)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        connection = str(Path(directory) / "connection.json")
        try:
            paired = subprocess.run([str(cli), "remote", "pair", "--url", f"http://127.0.0.1:{server.server_port}", "--connection", connection, "--json"], input="abc123\n", capture_output=True, text=True, timeout=10)
            assert paired.returncode == 0, paired.stderr
            assert "fixture-bearer-secret" not in paired.stdout + paired.stderr
            for command, extra, expected in [("project", [], "p"), ("session", ["--project", "p"], "s")]:
                result = subprocess.run([str(cli), command, "list", "--remote", connection, "--json"] + extra, capture_output=True, text=True, timeout=10)
                assert result.returncode == 0, result.stderr
                assert json.loads(result.stdout)[0]["id"] == expected
            value = json.loads(Path(connection).read_text())
            value["token"] = "wrong-token"
            Path(connection).write_text(json.dumps(value))
            denied = subprocess.run([str(cli), "project", "list", "--remote", connection, "--json"], capture_output=True, text=True, timeout=10)
            assert denied.returncode == 1
            assert denied.stdout == ""
            assert "fixture-bearer-secret" not in denied.stderr
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=5)

        base = [str(cli), "run", "--local", "--harness", shlex.quote(str(agent)), "--cwd", directory, "--prompt", "Please write hello.txt", "--json"]
        result = subprocess.run(base, capture_output=True, text=True, timeout=30)
        assert result.returncode == 1, result.stderr
        response = json.loads(result.stdout)
        assert response["permissions"] == 1, response
        assert response["stop_reason"] == "refusal", response
        assert "Permission denied" in response["text"], response
        assert not (Path(directory) / "hello.txt").exists()

        result = subprocess.run(base + ["--permission", "ask"], stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=10)
        assert result.returncode == 2, result
        assert result.stdout == ""

        master, slave = pty.openpty()
        process = subprocess.Popen(base + ["--permission", "ask"], stdin=slave, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        os.close(slave)
        try:
            deadline = time.monotonic() + 20
            diagnostics = b""
            while b"Allow this operation once?" not in diagnostics:
                if time.monotonic() > deadline:
                    raise AssertionError("No per-use permission prompt")
                ready, _, _ = select.select([process.stderr], [], [], 1)
                if ready:
                    chunk = os.read(process.stderr.fileno(), 4096)
                    assert chunk, diagnostics
                    diagnostics += chunk
            os.write(master, b"y\n")
            stdout, stderr = process.communicate(timeout=20)
            assert process.returncode == 0, (diagnostics, stderr)
            response = json.loads(stdout)
            assert response["stop_reason"] == "end_turn", response
            assert response["permissions"] == 1, response
            assert "Permission granted" in response["text"], response
        finally:
            os.close(master)
            if process.poll() is None:
                process.kill()
                process.communicate()

        env = dict(os.environ, DUNLIN_MOCK_HANG_INITIALIZE="1")
        result = subprocess.run(base + ["--timeout", "1"], env=env, capture_output=True, text=True, timeout=10)
        assert result.returncode == 124, result
        assert result.stdout == ""

        process = subprocess.Popen(base, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            time.sleep(1)
            process.send_signal(signal.SIGINT)
            stdout, stderr = process.communicate(timeout=10)
            assert process.returncode == 130, (process.returncode, stderr)
            assert stdout == ""
        finally:
            if process.poll() is None:
                process.kill()
                process.communicate()
    print("Headless CLI verified: pairing/catalog contracts, token redaction, ACP approval/denial, timeout and cancellation.")


if __name__ == "__main__":
    main()
