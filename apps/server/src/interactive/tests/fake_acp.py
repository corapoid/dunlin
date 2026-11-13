"""Local ACP fixture: no model, credentials or network. Process remains alive idle."""
import json
import os
import subprocess
import sys
import time

behavior = sys.argv[1]
with open("actor-starts", "a") as marker:
    marker.write(str(os.getpid()) + "\n")
with open("actor-pid", "w") as marker:
    marker.write(str(os.getpid()))
if behavior == "descendant":
    child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(3600)"])
    with open("descendant-pid", "w") as marker:
        marker.write(str(child.pid))
model, mode = "fixture-model", "plan"
mcp_config = None
mcp_process = None
def mcp_request(method, params, request_id):
    if behavior == "mcp-real":
        # Exercise the production facade over MCP JSON lines, including legal
        # reuse of a correlation ID after the previous reply.
        mcp_process.stdin.write(json.dumps({"jsonrpc":"2.0", "id":1, "method":method, "params":params})+"\n")
        mcp_process.stdin.flush()
        response = json.loads(mcp_process.stdout.readline())
        assert response["id"] == 1
        return {"result":response["result"]} if "result" in response else {"error":"facade rejected"}
    import socket
    import struct
    env = {item["name"]: item["value"] for item in mcp_config["env"]}
    def receive(peer, size):
        value = b""
        while len(value) < size:
            chunk = peer.recv(size-len(value))
            assert chunk
            value += chunk
        return value
    with socket.socket(socket.AF_UNIX) as peer:
        peer.settimeout(145)
        peer.connect(env["DUNLIN_MCP_SOCKET"])
        body = json.dumps({"token":env["DUNLIN_MCP_TOKEN"], "request_id":request_id,
                           "method":method, "params":params}).encode()
        peer.sendall(struct.pack(">I",len(body))+body)
        size = struct.unpack(">I",receive(peer,4))[0]
        assert size <= 256*1024
        return json.loads(receive(peer,size))
def send(value):
    data = json.dumps(value) + "\n"
    # Fragment across guardian ticks to exercise cancellation-safe frame buffering.
    sys.stdout.write(data[:10])
    sys.stdout.flush()
    time.sleep(0.01)
    sys.stdout.write(data[10:])
    sys.stdout.flush()
def options():
    return [{"id":"model", "category":"model", "currentValue":model},
            {"id":"mode", "category":"mode", "currentValue":mode}]
def reply(request, value):
    send({"jsonrpc":"2.0", "id":request["id"], "result":value})
for line in sys.stdin:
    request = json.loads(line)
    method, params = request["method"], request["params"]
    if method == "initialize":
        with open("received-initializes", "a") as marker:
            marker.write("initialize\n")
        assert params["clientCapabilities"]["terminal"] is False
        reply(request, {"protocolVersion":1})
    elif method == "session/new":
        if behavior in ("mcp", "mcp-real"):
            assert len(params["mcpServers"]) == 1
            mcp_config = params["mcpServers"][0]
            assert mcp_config["args"] == ["--mcp-tools"]
            assert {item["name"] for item in mcp_config["env"]} == {"DUNLIN_MCP_SOCKET", "DUNLIN_MCP_TOKEN"}
            if behavior == "mcp-real":
                env = {item["name"]:item["value"] for item in mcp_config["env"]}
                mcp_process = subprocess.Popen([mcp_config["command"],*mcp_config["args"]],env=env,
                    stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)
                mcp_process.stdin.write(json.dumps({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"fixture","version":"1"}}})+"\n")
                mcp_process.stdin.flush()
                assert json.loads(mcp_process.stdout.readline())["result"]["protocolVersion"] == "2025-06-18"
        else:
            assert params["mcpServers"] == []
        assert os.path.realpath(params["cwd"]) == os.getcwd()
        reply(request, {"sessionId":"fixture-interactive", "configOptions":options()})
    elif method == "session/set_config_option":
        if params["configId"] == "model": model = params["value"]
        if params["configId"] == "mode": mode = params["value"]
        if behavior == "wrong-model": model = "unapproved"
        reply(request, {"configOptions":options()})
    elif method == "session/prompt":
        text = params["prompt"][0]["text"]
        with open("received-prompts", "a") as log:
            log.write(text + "\n")
        if behavior in ("mcp", "mcp-real"):
            tools = mcp_request("tools/list", {}, "catalog")
            assert len(tools["result"]["tools"]) == 1
            name = tools["result"]["tools"][0]["name"]
            call = {"name":name,"arguments":{"query":"approved fixture"}}
            result = mcp_request("tools/call", call, "call-1")
            if behavior == "mcp":
                assert mcp_request("tools/call", call, "call-1") == result
                changed = {"name":name,"arguments":{"query":"changed"}}
                assert "error" in mcp_request("tools/call", changed, "call-1")
            with open("received-mcp-result", "w") as log:
                json.dump(result, log)
        if behavior in ("hang", "descendant"):
            while True: time.sleep(1)
        if behavior == "permission":
            send({"jsonrpc":"2.0", "id":"permission-1", "method":"session/request_permission", "params":{"sessionId":"fixture-interactive", "options":[{"kind":"allow_once","optionId":"once"}]}})
            denied = json.loads(sys.stdin.readline())
            assert denied["result"]["outcome"]["outcome"] == "cancelled"
            with open("permission-denied", "w"): pass
            continue
        if behavior == "oversize":
            sys.stdout.write("x" * (1024*1024+1)); sys.stdout.flush(); continue
        if behavior == "wrong-session":
            send({"jsonrpc":"2.0", "method":"session/update", "params":{"sessionId":"other-session", "update":{"sessionUpdate":"agent_message_chunk", "content":{"type":"text", "text":"private"}}}})
            continue
        send({"jsonrpc":"2.0", "method":"session/update", "params":{"sessionId":"fixture-interactive", "update":{"sessionUpdate":"agent_message_chunk", "content":{"type":"text", "text":"reply: " + text}}}})
        reply(request, {"stopReason":"end_turn"})
    else:
        raise AssertionError(method)
