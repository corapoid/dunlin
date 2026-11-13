import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { acceptSnapshot, decodeCatalog } from '../src/lib/catalog.ts';
import type { CatalogSnapshot } from '../src/lib/catalog.ts';

const snapshot: CatalogSnapshot = {
  protocol_version: 1, stream_id: 'fixture', sequence: 1, server_name: 'fixture server',
  user: { id: 'u', name: 'member', role: 'member' },
  projects: [{ id: 'p', name: 'project' }], project_id: 'p',
  sessions: [{ id: 's', title: 'session', updated_at: 1_000 }],
};

test('snapshot and resync wire contract', () => {
  assert.deepEqual(decodeCatalog(JSON.stringify({ type: 'snapshot', snapshot })), { type: 'snapshot', snapshot });
  assert.deepEqual(decodeCatalog('{"type":"resync_required"}'), { type: 'resync_required' });
});

test('the shared Rust/browser fixture matches the browser decoder', () => {
  const fixture = readFileSync(new URL('../../../tests/fixtures/web-catalog/snapshot.json', import.meta.url), 'utf8');
  assert.deepEqual(decodeCatalog(fixture), JSON.parse(fixture));
});

test('reject invalid versions, unsafe sequences and unrelated scope', () => {
  for (const changed of [
    { protocol_version: 2 }, { sequence: 0 }, { sequence: Number.MAX_SAFE_INTEGER + 1 }, { workspace_revision: -1 },
    { project_id: 'hidden' }, { project_id: null },
    { projects: [...snapshot.projects, ...snapshot.projects] },
    { sessions: [{ id: 's', title: 'session', updated_at: 'yesterday' }] },
    { projects: Array.from({ length: 501 }, (_, i) => ({ id: String(i), name: 'project' })) },
  ]) assert.throws(() => decodeCatalog(JSON.stringify({ type: 'snapshot', snapshot: { ...snapshot, ...changed } })));
  assert.throws(() => decodeCatalog('x'.repeat(2 * 1024 * 1024 + 1)));
});

test('reconnect accepts authoritative new stream; old sequence never overwrites newer state', () => {
  assert.equal(acceptSnapshot(snapshot, { ...snapshot, sequence: 1 }), false);
  assert.equal(acceptSnapshot(snapshot, { ...snapshot, sequence: 2 }), true);
  assert.equal(acceptSnapshot({ ...snapshot, sequence: 99 }, { ...snapshot, stream_id: 'new-stream' }), true);
});
