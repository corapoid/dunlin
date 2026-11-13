"""Local ACP permission fixture; only an explicit once decision writes a marker.

Usage: permission_peer.py normal|hold [--gate-resolution]
hold and the optional normal-mode gate wait for checkout/release-resolution
before consuming an already received decision.
"""
import json
import os
import select
import sys

behavior = sys.argv[1]
assert behavior in ("normal", "hold")
gated = "--gate-resolution" in sys.argv[2:]
session = "fixture-permissions"
model, mode = "fixture-model", "plan"
pending = None
decision = None
buffer = b""


def record(path, text):
    with open(path, "a", encoding="utf-8") as output:
        output.write(text + "\n")


def send(value):
    sys.stdout.write(json.dumps(value) + "\n")
    sys.stdout.flush()


def reply(request, result):
    send({"jsonrpc": "2.0", "id": request["id"], "result": result})


def update(value):
    send({"jsonrpc": "2.0", "method": "session/update",
          "params": {"sessionId": session, "update": value}})


def options():
    return [{"id": "model", "category": "model", "currentValue": model},
            {"id": "mode", "category": "mode", "currentValue": mode}]


with open("actor-pid", "w", encoding="utf-8") as marker:
    marker.write(str(os.getpid()))

while True:
    if pending is not None and decision is not None:
        if (behavior == "normal" and not gated) or os.path.exists("release-resolution"):
            outcome = decision["result"]["outcome"]
            selected = outcome["outcome"] == "selected"
            if selected:
                assert outcome["optionId"] in ("once", "deny"), "Persistent/unknown decision reached ACP"
            else:
                assert outcome["outcome"] == "cancelled"
            allowed = selected and outcome["optionId"] == "once"
            if allowed:
                # The path is chosen locally from a fixed allowlist, never from a
                # browser/peer-provided filesystem path.
                with open(pending["path"], "x", encoding="utf-8") as marker:
                    marker.write(pending["text"])
                record("performed-writes", pending["path"])
            update({"sessionUpdate": "tool_call_update", "toolCallId": "tool-1",
                    "status": "completed" if allowed else "failed",
                    "content": [{"type": "content", "content": {"type": "text",
                                 "text": "fixture marker written" if allowed else "fixture write denied"}}]})
            update({"sessionUpdate": "agent_message_chunk", "content": {
                "type": "text", "text": ("approved: " if allowed else "denied: ") + pending["text"]}})
            reply(pending["request"], {"stopReason": "end_turn"})
            pending = None
            decision = None
    ready, _, _ = select.select([sys.stdin.fileno()], [], [], 0.02)
    if not ready:
        continue
    data = os.read(sys.stdin.fileno(), 65536)
    if not data:
        break
    buffer += data
    while b"\n" in buffer:
        line, buffer = buffer.split(b"\n", 1)
        if not line:
            continue
        request = json.loads(line)
        if "method" not in request:
            assert request["id"] == "permission-1"
            if pending is None:
                record("ignored-decisions", json.dumps(request, sort_keys=True))
                continue
            assert decision is None
            record("received-decisions", json.dumps({"prompt": pending["text"], "response": request}, sort_keys=True))
            decision = request
            continue
        method, params = request["method"], request.get("params", {})
        if method == "initialize":
            assert params["clientCapabilities"]["terminal"] is False
            reply(request, {"protocolVersion": 1})
        elif method == "session/new":
            assert params["mcpServers"] == []
            assert os.path.realpath(params["cwd"]) == os.getcwd()
            reply(request, {"sessionId": session, "configOptions": options()})
        elif method == "session/set_config_option":
            if params["configId"] == "model":
                model = params["value"]
            elif params["configId"] == "mode":
                mode = params["value"]
            else:
                raise AssertionError(params)
            reply(request, {"configOptions": options()})
        elif method == "session/prompt":
            assert params["sessionId"] == session and pending is None
            text = params["prompt"][0]["text"]
            path = "denied.txt" if "deny" in text.lower() else "allowed.txt"
            record("received-prompts", text)
            pending = {"request": request, "text": text, "path": path}
            tool = {"toolCallId": "tool-1", "title": "Write fixture marker", "kind": "edit",
                    "status": "pending", "rawInput": {"path": path, "operation": "fixture-write"}}
            update({"sessionUpdate": "tool_call", **tool})
            # Reuse the remote identifiers in the next turn to exercise the relay's
            # per-request opaque approval identity rather than trusting peer IDs.
            send({"jsonrpc": "2.0", "id": "permission-1", "method": "session/request_permission",
                  "params": {"sessionId": session, "toolCall": tool, "options": [
                      {"kind": "allow_once", "optionId": "once", "name": "Allow once"},
                      {"kind": "allow_always", "optionId": "always", "name": "Always allow"},
                      {"kind": "reject_once", "optionId": "deny", "name": "Deny once"}]}})
        elif method == "session/cancel":
            assert params["sessionId"] == session
            record("received-cancels", session)
            if pending is not None:
                reply(pending["request"], {"stopReason": "cancelled"})
                pending = None
                decision = None
        else:
            raise AssertionError(method)
