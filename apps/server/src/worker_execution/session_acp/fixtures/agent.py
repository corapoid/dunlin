#!/usr/bin/python3
"""Actual pipe peer for engine tests. No sandbox/provider claim."""
import json
import os
from pathlib import Path
import signal
import sys
import time
import threading

behavior = sys.argv[1]
root = Path(sys.argv[2])
model = 'unset'
mode = 'unset'
last_id = 0
initialized = False
created = False
turns = 0
memory = None

def send(value):
    print(json.dumps(value), flush=True)

def update(text):
    send({'jsonrpc': '2.0', 'method': 'session/update', 'params': {
        'sessionId': 'persistent-fixture-session', 'update': {
            'sessionUpdate': 'agent_message_chunk', 'content': {'type': 'text', 'text': text}}}})

def options():
    return [{'id': 'model', 'category': 'model', 'currentValue': model},
            {'id': 'mode', 'category': 'mode', 'currentValue': mode}]

for line in sys.stdin:
    request = json.loads(line)
    if request.get('method') is None:
        # Unsupported broker request must be explicitly denied, never approved.
        assert request['id'] == 999
        assert request['result']['outcome']['outcome'] == 'cancelled'
        (root / 'denied').write_text('denied')
        continue
    rpc_id = request['id']
    assert rpc_id > last_id
    last_id = rpc_id
    method = request['method']
    params = request.get('params', {})
    with open(root / 'calls', 'a') as calls:
        calls.write(json.dumps({'method': method, 'id': rpc_id}) + '\n')
    if method == 'initialize':
        assert not initialized
        initialized = True
        assert params['clientCapabilities']['fs'] == {'readTextFile': False, 'writeTextFile': False}
        assert params['clientCapabilities']['terminal'] is False
        result = {'protocolVersion': 1}
    elif method == 'session/new':
        assert initialized and not created
        created = True
        assert params['mcpServers'] == [] and Path(params['cwd']) == root
        result = {'sessionId': 'persistent-fixture-session', 'configOptions': options()}
    elif method == 'session/set_config_option':
        assert created and turns == 0
        if params['configId'] == 'model':
            model = params['value']
        else:
            mode = params['value']
        result = {'configOptions': options()}
    elif method == 'session/prompt':
        assert created and model == 'fixture-model' and mode == 'fixture-mode'
        # Engine must await the actual caller's durable intent callback first.
        intents = [json.loads(line) for line in (root / 'intents').read_text().splitlines()]
        assert intents[-1]['rpc_id'] == rpc_id
        text = params['prompt'][0]['text']
        assert intents[-1]['prompt'] == text
        turns += 1
        if turns == 1:
            assert 'literal-instructions' in text and 'literal-skill' in text
            assert intents[-1]['first_turn'] is True
            memory = text.split('nonce:')[-1]
        else:
            assert 'literal-instructions' not in text and 'literal-skill' not in text
            assert intents[-1]['first_turn'] is False
        with open(root / 'effects', 'ab', buffering=0) as effects:
            effects.write(b'x\n')
        if behavior == 'slow':
            time.sleep(.15)
        if behavior == 'oversized':
            update('x' * (70 * 1024))
        elif behavior == 'permission':
            send({'jsonrpc': '2.0', 'id': 999, 'method': 'session/request_permission',
                  'params': {'sessionId': 'persistent-fixture-session', 'options': []}})
            continue
        elif behavior == 'chunks':
            update('界' * 2000)
        else:
            update('remembered:' + memory)
        result = {'stopReason': 'end_turn'}
    else:
        raise AssertionError('unsupported method')
    send({'jsonrpc': '2.0', 'id': rpc_id, 'result': result})
    if behavior == 'idle-during-intent' and method == 'session/set_config_option' and params['configId'] == 'mode':
        def late_idle_update():
            time.sleep(.1)
            # Partial frame begins while caller is awaiting durable intent.
            sys.stdout.write('{"jsonrpc":"2.0",')
            sys.stdout.flush()
            time.sleep(.1)
            sys.stdout.write('"method":"session/update","params":{"sessionId":"persistent-fixture-session","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"old idle output"}}}}\n')
            sys.stdout.flush()
        threading.Thread(target=late_idle_update, daemon=True).start()
    if behavior == 'blocked' and method == 'session/set_config_option' and params['configId'] == 'mode':
        (root / 'blocked-ready').write_text('ready')
        while True:
            signal.pause()
    if behavior == 'idle-update' and method == 'session/prompt':
        time.sleep(.1)
        update('late old output')
