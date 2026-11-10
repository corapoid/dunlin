"""Deterministic local ACP fixture. Never invokes a model or network service."""
import json
import os
import sys
import time

behavior = sys.argv[1]

def send(message, fragmented=False):
    data = json.dumps(message) + "\n"
    if fragmented:
        sys.stdout.write(data[:17])
        sys.stdout.flush()
        time.sleep(0.25)  # Cross the runner's authorization timer tick.
        sys.stdout.write(data[17:])
    else:
        sys.stdout.write(data)
    sys.stdout.flush()

def reply(request, result):
    send({"jsonrpc": "2.0", "id": request["id"], "result": result}, True)

def call(method, params):
    send({"jsonrpc":"2.0", "id":"client-request", "method":method,
          "params":{"sessionId":"fixture-session", **params}})
    result = json.loads(sys.stdin.readline())
    assert result["id"] == "client-request", result
    return result

def options(model="fixture-model"):
    return [{"id":"model", "category":"model", "currentValue":model,
             "options":[{"value":"fixture-model"}]},
            {"id":"mode", "category":"mode", "currentValue":"build",
             "options":[{"value":"build"}]}]

for line in sys.stdin:
    request = json.loads(line)
    method = request["method"]
    p = request["params"]
    if method == "initialize":
        reply(request, {"protocolVersion":1})
    elif method == "session/new":
        assert p["mcpServers"] == []
        assert os.path.realpath(p["cwd"]) == os.getcwd()
        reply(request, {"sessionId":"fixture-session", "configOptions":options()})
    elif method == "session/set_config_option":
        assert (p["configId"], p["value"]) in [("model","fixture-model"),("mode","build")]
        wrong = behavior == "wrong_model" or (behavior == "mode_changes_model" and p["configId"] == "mode")
        reply(request, {"configOptions":options("unapproved" if wrong else "fixture-model")})
    elif method == "session/prompt":
        if behavior in ("wrong_model", "mode_changes_model"):
            open("prompt-after-rejection", "w").close()
            raise AssertionError("Unapproved profile reached prompt")
        if behavior == "hang":
            with open("started", "w") as marker:
                marker.write(str(os.getpid()))
            while True:
                time.sleep(1)
        if behavior == "changed_model":
            send({"jsonrpc":"2.0", "method":"session/update", "params":{
                "sessionId":"fixture-session", "update":{"sessionUpdate":"config_option_update", "configOptions":options("unapproved")}}})
            continue
        if behavior == "escape":
            call("fs/read_text_file", {"path":"/etc/passwd"})
            continue
        permission = call("session/request_permission", {"options":[{"kind":"allow_once","optionId":"once"}]})
        if behavior == "permission":
            assert permission["result"]["outcome"]["outcome"] == "cancelled"
            continue
        assert permission["result"]["outcome"]["optionId"] == "once"
        content = call("fs/write_text_file", {"path":"result.txt", "content":"one\ntwo\nthree\n"})
        assert "error" not in content
        content = call("fs/read_text_file", {"path":"result.txt", "line":2, "limit":1})
        assert content["result"]["content"] == "two\n"
        send({"jsonrpc":"2.0", "method":"session/update", "params":{
            "sessionId":"fixture-session", "update":{"sessionUpdate":"agent_message_chunk", "content":{"type":"text","text":"fixture review"}}}})
        reply(request, {"stopReason":"end_turn"})
    else:
        raise AssertionError(method)
