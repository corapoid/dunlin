#!/usr/bin/python3
"""Controlled ACP process for real namespace tests. No provider or credentials."""
import json
import os
import signal
from pathlib import Path
import sys
import time

behavior = sys.argv[1]
common = Path(sys.argv[2]) if len(sys.argv) > 2 else None
model = 'fixture-model'
mode = 'fixture-mode'

def options():
    return [{'id': 'model', 'category': 'model', 'currentValue': model},
            {'id': 'mode', 'category': 'mode', 'currentValue': mode}]

def send(value):
    print(json.dumps(value), flush=True)

for line in sys.stdin:
    request = json.loads(line)
    method = request.get('method')
    params = request.get('params', {})
    if method == 'initialize':
        assert params['clientCapabilities']['fs'] == {'readTextFile': False, 'writeTextFile': False}
        assert params['clientCapabilities']['terminal'] is False
        assert not any(name in os.environ for name in ['GH_TOKEN', 'GITHUB_TOKEN', 'SSH_AUTH_SOCK', 'OPENAI_API_KEY', 'ANTHROPIC_API_KEY', 'DUNLIN_TEST_AMBIENT_SECRET'])
        result = {'protocolVersion': 1}
    elif method == 'session/new':
        assert params['mcpServers'] == []
        assert Path(params['cwd']).resolve() == Path.cwd()
        result = {'sessionId': 'fixture-session', 'configOptions': options()}
    elif method == 'session/set_config_option':
        if params['configId'] == 'model':
            model = params['value']
        else:
            mode = 'unsupported-mode' if behavior == 'bad-mode' else params['value']
        result = {'configOptions': options()}
    elif method == 'session/prompt':
        text = params['prompt'][0]['text']
        assert 'literal-instructions' in text and 'literal-skill' in text
        Path('received-prompt').write_text(text)
        Path('negotiated').write_text(json.dumps({'model': model, 'mode': mode}))
        if behavior == 'permission':
            send({'jsonrpc': '2.0', 'id': 90, 'method': 'session/request_permission', 'params': {'sessionId': 'fixture-session', 'toolCall': {'toolCallId': 'tool'}, 'options': [{'optionId': 'allow', 'kind': 'allow_once', 'name': 'Allow'}]}})
            response = json.loads(sys.stdin.readline())
            if response.get('result', {}).get('outcome', {}).get('outcome') == 'selected':
                Path('UNSAFE-approved').write_text('must never exist')
            continue
        with open('effects', 'ab', buffering=0) as effect:
            effect.write(b'x\n')
        if common is not None:
            # Common Git metadata is writable only through an admitted typed
            # metadata mount; the other worktree's source remains invisible.
            (common / 'worker-write').write_text('allowed')
            assert not (common.parent / 'source-private').exists()
        if behavior == 'hang':
            held = open((common / 'objects' / 'held-effect') if common else Path('held-effect'), 'ab', buffering=0)
            if os.fork() == 0:
                while True:
                    held.write(b'x')
                    time.sleep(.02)
            Path('held-ready').write_text('ready')
            send({'jsonrpc': '2.0', 'method': 'session/update', 'params': {'sessionId': 'fixture-session', 'update': {'sessionUpdate': 'agent_message_chunk', 'content': {'type': 'text', 'text': 'started'}}}})
            # Child holds stdout + writable FD while this process blocks stdin.
            sys.stdin.readline()
            continue
        if behavior == 'large':
            message = 'x' * (16 * 1024 + 1)
        else:
            message = 'one confirmed fixture turn'
        send({'jsonrpc': '2.0', 'method': 'session/update', 'params': {'sessionId': 'fixture-session', 'update': {'sessionUpdate': 'agent_message_chunk', 'content': {'type': 'text', 'text': message}}}})
        result = {'stopReason': 'end_turn'}
    else:
        raise AssertionError('unexpected request')
    send({'jsonrpc': '2.0', 'id': request['id'], 'result': result})

    if behavior == 'blocked-write' and method == 'session/set_config_option' and params['configId'] == 'mode':
        held = open((common / 'objects' / 'held-effect') if common else Path('held-effect'), 'ab', buffering=0)
        if os.fork() == 0:
            while True:
                held.write(b'x')
                time.sleep(.02)
        Path('held-ready').write_text('ready')
        while True:
            signal.pause()
