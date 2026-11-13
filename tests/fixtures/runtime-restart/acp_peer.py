"""Offline ACP crash fixture; a pending prompt deliberately survives stdin EOF.

The production relay/sandbox must own termination after a crash. This fixture
sets no parent-death signal, watches no server PID, and never self-cleans a
pending prompt when the relay's pipe disappears.
"""
import json
import os
import sys
import time

model, mode = "fixture-model", "plan"
pending = None


def record(path, text):
    with open(path, "a", encoding="utf-8") as output:
        output.write(text + "\n")


def send(value):
    print(json.dumps(value), flush=True)


def reply(request, value):
    send({"jsonrpc": "2.0", "id": request["id"], "result": value})


def options():
    return [{"id": "model", "category": "model", "currentValue": model},
            {"id": "mode", "category": "mode", "currentValue": mode}]


record("peer-starts", str(os.getpid()))
for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    params = request.get("params", {})
    if method is None:
        assert pending is not None and request["id"] == "permission-restart"
        record("received-decisions", json.dumps(request, sort_keys=True))
        if request.get("result", {}).get("outcome", {}).get("optionId") == "once":
            record("permission-effects", pending["params"]["prompt"][0]["text"])
        reply(pending, {"stopReason": "end_turn"})
        pending = None
    elif method == "initialize":
        assert params["clientCapabilities"]["terminal"] is False
        reply(request, {"protocolVersion": 1})
    elif method == "session/new":
        assert params["mcpServers"] == []
        reply(request, {"sessionId": "restart-peer", "configOptions": options()})
    elif method == "session/set_config_option":
        if params["configId"] == "model":
            model = params["value"]
        elif params["configId"] == "mode":
            mode = params["value"]
        else:
            raise AssertionError(params)
        reply(request, {"configOptions": options()})
    elif method == "session/prompt":
        text = params["prompt"][0]["text"]
        record("received-prompts", text)
        # This harmless file record stands in for an external effect which has
        # already happened, before the relay can record a terminal turn outcome.
        record("performed-effects", text)
        if text == "ambiguous-effect-before-crash":
            pending = request
            tool = {"toolCallId": "tool-restart", "title": "Controlled permission", "kind": "edit", "status": "pending", "rawInput": {"fixture": "permission-effect"}}
            send({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "restart-peer", "update": {"sessionUpdate": "tool_call", **tool}}})
            send({"jsonrpc": "2.0", "id": "permission-restart", "method": "session/request_permission", "params": {"sessionId": "restart-peer", "toolCall": tool, "options": [{"kind": "allow_once", "optionId": "once", "name": "Allow once"}, {"kind": "reject_once", "optionId": "deny", "name": "Deny once"}]}})
        else:
            send({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "restart-peer", "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "fresh-completed"}}}})
            reply(request, {"stopReason": "end_turn"})
    elif method == "session/cancel":
        if pending is not None:
            reply(pending, {"stopReason": "cancelled"})
            pending = None
    else:
        raise AssertionError(method)

if pending is not None:
    while True:
        time.sleep(1)
