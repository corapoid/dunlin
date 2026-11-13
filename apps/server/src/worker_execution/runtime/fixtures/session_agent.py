#!/usr/bin/python3
"""Controlled persistent ACP peer, used only in real sandbox acceptance."""
import json
import os
from pathlib import Path
import sys
import subprocess
import socket
import struct
import time

behavior = sys.argv[1]
model = 'fixture-model'
mode = 'fixture-mode'
initialized = False
created = False
last_id = 0
memory = None
turns = 0
mcp = None
mcp_id = 0
mcp_name = None

def send(value):
    print(json.dumps(value), flush=True)

def mcp_rpc(method, params):
    global mcp_id
    mcp_id += 1
    mcp.stdin.write(json.dumps({'jsonrpc':'2.0','id':mcp_id,'method':method,'params':params}) + '\n')
    mcp.stdin.flush()
    reply = json.loads(mcp.stdout.readline())
    assert reply['id'] == mcp_id and 'error' not in reply
    return reply['result']

def options():
    return [{'id': 'model', 'category': 'model', 'currentValue': model},
            {'id': 'mode', 'category': 'mode', 'currentValue': mode}]

for line in sys.stdin:
    request = json.loads(line)
    rpc_id = request['id']
    assert rpc_id > last_id
    last_id = rpc_id
    method = request['method']
    params = request.get('params', {})
    with open('session-calls', 'a') as calls:
        calls.write(json.dumps({'method': method, 'id': rpc_id, 'pid': os.getpid()}) + '\n')
    if method == 'initialize':
        assert not initialized
        initialized = True
        assert params['clientCapabilities']['terminal'] is False
        assert params['clientCapabilities']['fs'] == {'readTextFile': False, 'writeTextFile': False}
        assert not any(key in os.environ for key in ['GH_TOKEN', 'GITHUB_TOKEN', 'SSH_AUTH_SOCK', 'OPENAI_API_KEY', 'ANTHROPIC_API_KEY', 'DUNLIN_TEST_AMBIENT_SECRET'])
        result = {'protocolVersion': 1}
    elif method == 'session/new':
        assert initialized and not created
        created = True
        assert Path(params['cwd']).resolve() == Path.cwd()
        if behavior.startswith('native-mcp'):
            assert len(params['mcpServers']) == 1
            server = params['mcpServers'][0]
            assert server['args'] == ['--mcp-tools']
            env = dict(os.environ)
            extra = {item['name']:item['value'] for item in server['env']}
            assert set(extra) == {'DUNLIN_MCP_SOCKET','DUNLIN_MCP_TOKEN'}
            assert len(extra['DUNLIN_MCP_TOKEN']) <= 128
            env.update(extra)
            mcp = subprocess.Popen([server['command']] + server['args'], env=env,
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1)
            mcp_rpc('initialize', {'protocolVersion':'2025-06-18','capabilities':{},'clientInfo':{'name':'native-fixture','version':'1'}})
            tools = mcp_rpc('tools/list', {})['tools']
            assert len(tools) == 1
            mcp_name = tools[0]['name']
        else:
            assert params['mcpServers'] == []
        # A setsid descendant with a held writable FD must die on idle revoke,
        # not merely when another prompt happens to reach the engine.
        held = open('session-held-effect', 'ab', buffering=0)
        if os.fork() == 0:
            os.setsid()
            while True:
                held.write(b'x')
                time.sleep(.02)
        held.close()
        result = {'sessionId': 'persistent-kernel-session', 'configOptions': options()}
    elif method == 'session/set_config_option':
        assert created and turns == 0
        if params['configId'] == 'model':
            model = params['value']
        else:
            mode = params['value']
        result = {'configOptions': options()}
    elif method == 'session/prompt':
        assert created and params['sessionId'] == 'persistent-kernel-session'
        text = params['prompt'][0]['text']
        turns += 1
        if turns == 1:
            assert 'literal-instructions' in text and 'literal-skill' in text
            memory = text.split('nonce:')[-1]
        else:
            assert 'literal-instructions' not in text and 'literal-skill' not in text
        if behavior == 'native-mcp-disconnect':
            packet = json.dumps({'token':extra['DUNLIN_MCP_TOKEN'],'request_id':'drop-once',
                'method':'tools/call','params':{'name':mcp_name,'arguments':{'query':'native fixture'}}}).encode()
            stream = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            stream.connect(extra['DUNLIN_MCP_SOCKET'])
            stream.sendall(struct.pack('>I',len(packet)) + packet)
            stream.close()
            while True:
                time.sleep(.02)
        if behavior in ['native-mcp', 'native-mcp-deny', 'native-mcp-tool-failed']:
            result = mcp_rpc('tools/call', {'name':mcp_name,'arguments':{'query':'native fixture'}})
            if behavior == 'native-mcp-deny':
                assert result['isError'] is True and 'denied' in result['content'][0]['text']
            elif behavior == 'native-mcp-tool-failed':
                assert result['isError'] is True and 'reported failure' in result['content'][0]['text']
            else:
                assert result['content'][0]['text'] == 'native broker result'
        if behavior not in ['native-mcp-deny', 'native-mcp-tool-failed']:
            with open('effects', 'ab', buffering=0) as effects:
                effects.write(b'x\n')
        cancelling = text == 'Start cancellation fixture'
        crashing = text.endswith('Start crash fixture')
        send({'jsonrpc': '2.0', 'method': 'session/update', 'params': {
            'sessionId': 'persistent-kernel-session', 'update': {
                'sessionUpdate': 'agent_message_chunk',
                'content': {'type': 'text', 'text': 'active-cancel' if cancelling else ('active-crash' if crashing else 'remembered:' + memory)}}}})
        if cancelling or crashing:
            while True:
                time.sleep(.02)
        result = {'stopReason': 'end_turn'}
    else:
        raise AssertionError('unexpected ACP method')
    send({'jsonrpc': '2.0', 'id': rpc_id, 'result': result})
