import { test } from 'node:test';
import assert from 'node:assert/strict';
import { liveEndpoint, validateEndpoint } from '../src/lib/transport.ts';

test('remote transport is Tailscale-only and local HTTP/WS is allowed', () => {
  for (const value of ['https://build.tail123.ts.net', 'wss://build.tail123.ts.net', 'http://127.0.0.1:8787', 'ws://localhost:8787', 'http://[::1]:8787']) {
    assert.doesNotThrow(() => validateEndpoint(value), value);
  }
  for (const value of ['https://example.com', 'https://192.168.1.5', 'http://build.tail123.ts.net', 'ws://build.tail123.ts.net', 'https://build.tail123.ts.net.evil.test', 'https://ts.net', 'https://build..ts.net', 'file:///etc/passwd', 'https://user:secret@build.tail123.ts.net', 'http://localhost:8787/?token=secret', 'http://localhost:8787/#secret', 'https://build.tail123.ts.net/path']) {
    assert.throws(() => validateEndpoint(value), value);
  }
});

test('live URL has no credentials or token and preserves secure transport', () => {
  assert.equal(liveEndpoint('https://build.tail123.ts.net'), 'wss://build.tail123.ts.net/v1/web/live');
  assert.equal(liveEndpoint('http://127.0.0.1:8787'), 'ws://127.0.0.1:8787/v1/web/live');
});
