#!/usr/bin/env python3
"""Verify that OpenCode's ACP server connects Dunlin's vault MCP proxy.

This smoke does not send a model prompt. It checks the part that is easy to get
wrong and expensive to discover in normal use:

  1. Dunlin passes a stdio MCP server in ACP session/new.
  2. OpenCode spawns `dunlin --mcp-vault`.
  3. The proxy reaches the local vault socket and completes its handshake.

It uses a mock vault service, so no Keychain item or real credential is touched.
Requires `opencode` and a debug build of Dunlin.
"""

from __future__ import annotations

import json
import os
import select
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DUNLIN = os.environ.get("DUNLIN_BIN", os.path.join(ROOT, "target/debug/dunlin"))
OPENCODE = os.environ.get("OPENCODE_BIN", "opencode")


def read_frame(connection: socket.socket):
    header = connection.recv(4)
    if len(header) < 4:
        return None
    length = struct.unpack(">I", header)[0]
    data = b""
    while len(data) < length:
        part = connection.recv(length - len(data))
        if not part:
            return None
        data += part
    return json.loads(data)


def write_frame(connection: socket.socket, value):
    data = json.dumps(value).encode("utf-8")
    connection.sendall(struct.pack(">I", len(data)) + data)


def mock_vault(path: str, events: list, ready: threading.Event):
    listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    listener.bind(path)
    listener.listen(1)
    try:
        ready.set()
        connection, _ = listener.accept()
        hello = read_frame(connection)
        events.append(hello)
        write_frame(connection, {"type": "hello", "ok": True, "message": None})
        while True:
            request = read_frame(connection)
            if request is None:
                break
            events.append(request)
            if request.get("type") == "list":
                write_frame(connection, {"type": "list", "secrets": []})
    finally:
        listener.close()


def read_acp_response(process, request_id: int, timeout: float):
    deadline = time.time() + timeout
    while time.time() < deadline:
        remaining = max(0.0, deadline - time.time())
        ready, _, _ = select.select([process.stdout], [], [], remaining)
        if not ready:
            return None
        line = process.stdout.readline()
        if not line:
            return None
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        if message.get("id") == request_id:
            return message
    return None


def main() -> int:
    if not os.path.isfile(DUNLIN):
        print(f"missing debug Dunlin binary: {DUNLIN}", file=sys.stderr)
        return 2
    if not subprocess.run(
        ["sh", "-c", f"command -v {OPENCODE!r}"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    ).returncode == 0:
        print(f"{OPENCODE} is not installed; skipping OpenCode ACP smoke")
        return 0

    with tempfile.TemporaryDirectory(prefix="dunlin-vault-smoke-") as root:
        socket_path = os.path.join(root, "vault.sock")
        events: list = []
        ready = threading.Event()
        server = threading.Thread(
            target=mock_vault, args=(socket_path, events, ready), daemon=True
        )
        server.start()
        if not ready.wait(5):
            print("mock vault service did not start", file=sys.stderr)
            return 2

        cwd = os.path.join(root, "project")
        os.makedirs(cwd)
        process = subprocess.Popen(
            [OPENCODE, "acp"],
            cwd=cwd,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
        )
        try:
            def send(value):
                process.stdin.write(json.dumps(value) + "\n")
                process.stdin.flush()

            send(
                {
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "initialize",
                    "params": {
                        "protocolVersion": 1,
                        "clientCapabilities": {},
                        "clientInfo": {"name": "DunlinVaultSmoke", "version": "1"},
                    },
                }
            )
            initialized = read_acp_response(process, 1, 20)
            if not initialized or "result" not in initialized:
                print(f"ACP initialize failed: {initialized}", file=sys.stderr)
                return 1

            send(
                {
                    "jsonrpc": "2.0",
                    "id": 2,
                    "method": "session/new",
                    "params": {
                        "cwd": cwd,
                        "mcpServers": [
                            {
                                "name": "dunlin-secrets",
                                "command": os.path.abspath(DUNLIN),
                                "args": ["--mcp-vault"],
                                "env": [
                                    {
                                        "name": "DUNLIN_VAULT_SOCKET",
                                        "value": socket_path,
                                    },
                                    {
                                        "name": "DUNLIN_VAULT_TOKEN",
                                        "value": "vault-smoke-token",
                                    },
                                ],
                            }
                        ],
                    },
                }
            )
            session = read_acp_response(process, 2, 30)
            if not session or "result" not in session:
                print(f"ACP session/new failed: {session}", file=sys.stderr)
                return 1
            if not any(
                event.get("type") == "hello"
                and event.get("token") == "vault-smoke-token"
                for event in events
                if isinstance(event, dict)
            ):
                print(
                    "OpenCode did not connect the vault MCP proxy", file=sys.stderr
                )
                return 1
            print("OpenCode ACP vault MCP hand-off smoke passed")
            return 0
        finally:
            process.kill()
            try:
                process.communicate(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()


if __name__ == "__main__":
    raise SystemExit(main())
