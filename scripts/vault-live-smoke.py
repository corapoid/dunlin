#!/usr/bin/env python3
"""Live OpenCode acceptance for the vault MCP broker.

This sends one real model turn. It costs provider tokens, but it does not use a
real credential: the mock vault returns a fixed 200 response for the requested
`api.github.com` call. The turn succeeds if the model calls the vault MCP tool
`secrets_request` and the mock broker receives that request.

Requires an authenticated OpenCode and a debug Dunlin build.
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
MODEL = os.environ.get("VAULT_LIVE_MODEL", "github-copilot/claude-sonnet-5")
TIMEOUT = float(os.environ.get("VAULT_LIVE_TIMEOUT", "180"))

SUMMARY = {
    "alias": "github-work",
    "label": "GitHub (live smoke)",
    "description": "Mock credential for api.github.com only.",
    "hosts": ["api.github.com"],
    "methods": ["GET"],
    "recipes": [],
    "expires_at": None,
    "expires_in_secs": None,
    "expired": False,
    "auto_approve": True,
}


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
            kind = request.get("type")
            if kind == "list":
                write_frame(connection, {"type": "list", "secrets": [SUMMARY]})
            elif kind == "http":
                write_frame(
                    connection,
                    {
                        "type": "http",
                        "output": {
                            "status": 200,
                            "content_type": "application/json",
                            "body": '{"login":"vault-live-smoke"}',
                            "truncated": False,
                        },
                    },
                )
            elif kind == "run":
                write_frame(
                    connection,
                    {
                        "type": "run",
                        "output": {
                            "exit_code": 0,
                            "signal": None,
                            "stdout": "vault-live-smoke\n",
                            "stderr": "",
                            "truncated": False,
                            "timed_out": False,
                        },
                    },
                )
    finally:
        listener.close()


def main() -> int:
    if not os.path.isfile(DUNLIN):
        print(f"missing debug Dunlin binary: {DUNLIN}", file=sys.stderr)
        return 2
    if (
        subprocess.run(
            ["sh", "-c", f"command -v {OPENCODE!r}"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        ).returncode
        != 0
    ):
        print(f"{OPENCODE} is not installed; skipping live acceptance")
        return 0

    with tempfile.TemporaryDirectory(prefix="dunlin-vault-live-") as root:
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
        pending: dict = {}
        allow_choice: dict = {}
        agent_text: list = []
        prompt_response = None

        def send(value):
            process.stdin.write(json.dumps(value) + "\n")
            process.stdin.flush()

        def handle_agent_request(message):
            method = message.get("method")
            request_id = message.get("id")
            if method == "session/request_permission":
                options = message.get("params", {}).get("options", [])
                selected = next(
                    (
                        option
                        for option in options
                        if option.get("kind") in ("allow_once", "allow_always")
                    ),
                    None,
                )
                allow_choice["option"] = selected
                send(
                    {
                        "jsonrpc": "2.0",
                        "id": request_id,
                        "result": {
                            "outcome": (
                                {
                                    "outcome": "selected",
                                    "optionId": selected["optionId"],
                                }
                                if selected
                                else {"outcome": "cancelled"}
                            )
                        },
                    }
                )
            else:
                # The live prompt only needs the MCP server. Do not pretend to
                # support filesystem or terminal ACP client methods.
                send(
                    {
                        "jsonrpc": "2.0",
                        "id": request_id,
                        "error": {"code": -32601, "message": "unsupported client method"},
                    }
                )

        def read_messages_until(deadline, stop_on_http=False):
            nonlocal prompt_response
            while time.time() < deadline:
                remaining = max(0.0, deadline - time.time())
                ready_fds, _, _ = select.select([process.stdout], [], [], remaining)
                if not ready_fds:
                    return False
                line = process.stdout.readline()
                if not line:
                    return False
                try:
                    message = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if "method" in message and "id" in message:
                    handle_agent_request(message)
                    continue
                if "id" in message:
                    pending[message["id"]] = message
                    if message["id"] == 4:
                        prompt_response = message
                    continue
                # Notifications: agent message chunks are useful evidence.
                update = message.get("params", {}).get("update", {})
                if update.get("sessionUpdate") == "agent_message_chunk":
                    content = update.get("content", {})
                    if content.get("type") == "text":
                        agent_text.append(content.get("text", ""))
                if stop_on_http and any(
                    isinstance(event, dict) and event.get("type") == "http"
                    for event in events
                ):
                    return True
            return False

        try:
            send(
                {
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "initialize",
                    "params": {
                        "protocolVersion": 1,
                        "clientCapabilities": {},
                        "clientInfo": {"name": "DunlinVaultLiveSmoke", "version": "1"},
                    },
                }
            )
            deadline = time.time() + 20
            while time.time() < deadline and 1 not in pending:
                read_messages_until(min(deadline, time.time() + 1))
            if 1 not in pending or "result" not in pending[1]:
                print(
                    f"ACP initialize failed: {pending.get(1)}", file=sys.stderr
                )
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
                                        "value": "vault-live-token",
                                    },
                                ],
                            }
                        ],
                    },
                }
            )
            deadline = time.time() + 30
            while time.time() < deadline and 2 not in pending:
                read_messages_until(min(deadline, time.time() + 1))
            if 2 not in pending or "result" not in pending[2]:
                print(f"ACP session/new failed: {pending.get(2)}", file=sys.stderr)
                return 1
            session_id = pending[2]["result"]["sessionId"]
            if not any(
                isinstance(event, dict)
                and event.get("type") == "hello"
                and event.get("token") == "vault-live-token"
                for event in events
            ):
                print("OpenCode did not connect the vault MCP proxy", file=sys.stderr)
                return 1

            send(
                {
                    "jsonrpc": "2.0",
                    "id": 3,
                    "method": "session/set_config_option",
                    "params": {
                        "sessionId": session_id,
                        "configId": "model",
                        "value": MODEL,
                    },
                }
            )
            deadline = time.time() + 30
            while time.time() < deadline and 3 not in pending:
                read_messages_until(min(deadline, time.time() + 1))
            if 3 not in pending or "result" not in pending[3]:
                print(
                    f"ACP could not select {MODEL}: {pending.get(3)}",
                    file=sys.stderr,
                )
                return 1

            send(
                {
                    "jsonrpc": "2.0",
                    "id": 4,
                    "method": "session/prompt",
                    "params": {
                        "sessionId": session_id,
                        "prompt": [
                            {
                                "type": "text",
                                "text": (
                                    "Use the dunlin-secrets MCP tool secrets_request "
                                    "with secret \"github-work\", method \"GET\", and "
                                    "url \"https://api.github.com/user\". Do not ask me "
                                    "for any key. The tool result is mock JSON; report "
                                    "the login value from it."
                                ),
                            }
                        ],
                    },
                }
            )

            deadline = time.time() + TIMEOUT
            got_http = False
            while time.time() < deadline:
                read_messages_until(min(deadline, time.time() + 1), stop_on_http=True)
                got_http = any(
                    isinstance(event, dict) and event.get("type") == "http"
                    for event in events
                )
                if got_http and prompt_response is not None:
                    break
                if prompt_response is not None and not got_http:
                    break
            if not got_http:
                print(
                    "the model did not call secrets_request; prompt response: "
                    f"{prompt_response}; agent text: {''.join(agent_text)[-500:]}",
                    file=sys.stderr,
                )
                return 1
            http_event = next(
                event
                for event in events
                if isinstance(event, dict) and event.get("type") == "http"
            )
            print(
                "Live OpenCode vault acceptance passed: "
                f"{http_event.get('method')} {http_event.get('url')}"
            )
            return 0
        finally:
            process.kill()
            try:
                process.communicate(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()


if __name__ == "__main__":
    raise SystemExit(main())
