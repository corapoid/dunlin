"""Local ACP cancellation peer. No model, credentials, network, threads or tools.

Usage: control_peer.py cooperative|ignore-cancel [--gate-cancel]
The optional gate waits for checkout/release-cancel before acknowledging cancellation.
"""
import json
import os
import select
import sys

behavior = sys.argv[1]
assert behavior in ("cooperative", "ignore-cancel")
gated = "--gate-cancel" in sys.argv[2:]
session = "fixture-controls"
model, mode = "fixture-model", "plan"
pending = None
cancelled = False
buffer = b""


def record(path, text):
    with open(path, "a", encoding="utf-8") as output:
        output.write(text + "\n")


def send(value):
    sys.stdout.write(json.dumps(value) + "\n")
    sys.stdout.flush()


def reply(request, result):
    send({"jsonrpc": "2.0", "id": request["id"], "result": result})


def chunk(text):
    send({"jsonrpc": "2.0", "method": "session/update", "params": {
        "sessionId": session, "update": {"sessionUpdate": "agent_message_chunk",
        "content": {"type": "text", "text": text}}}})


def options():
    return [{"id": "model", "category": "model", "currentValue": model},
            {"id": "mode", "category": "mode", "currentValue": mode}]


with open("actor-pid", "w", encoding="utf-8") as marker:
    marker.write(str(os.getpid()))

while True:
    if pending is not None and cancelled and behavior == "cooperative":
        if not gated or os.path.exists("release-cancel"):
            record("control-events", "cancelled-result")
            reply(pending, {"stopReason": "cancelled"})
            pending = None
            cancelled = False
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
            assert params["sessionId"] == session
            text = params["prompt"][0]["text"]
            record("received-prompts", text)
            record("control-events", "prompt:" + text)
            # A queued request must never reach ACP while cancellation is unsettled.
            assert pending is None, "Prompt arrived before previous request settled"
            if text.startswith("hold"):
                pending = request
                chunk("started: " + text)
            else:
                chunk("reply: " + text)
                record("control-events", "completed")
                # Unknown ACP result fields remain peer data, never local evidence
                # of whether the runtime actually sent a cancellation notification.
                reply(request, {"stopReason": "end_turn",
                                "_dunlinCancellationEvidence": "not_sent"})
        elif method == "session/cancel":
            assert params["sessionId"] == session
            record("received-cancels", params["sessionId"])
            record("control-events", "cancel")
            assert pending is not None, "No pending prompt to cancel"
            cancelled = True
        else:
            raise AssertionError(method)
