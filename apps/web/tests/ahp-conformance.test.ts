import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { chat, reduceChat } from '../src/lib/ahp.ts';
import type { Json } from '../src/lib/ahp.ts';
const traces: Json[] = JSON.parse(readFileSync(new URL('./fixtures/ahp-reducer-traces.json', import.meta.url), 'utf8'));
const reference: Json = JSON.parse(readFileSync(new URL('./fixtures/ahp-reducer-expected.json', import.meta.url), 'utf8'));
function wire(value: unknown): unknown { return JSON.parse(JSON.stringify(value)); }
// This golden is generated and independently verified by
// dunlin-session-runtime::projection::frontend_oracle using unmodified ahp 1.0.0.
for (const trace of traces) {
  test(`${trace.knownDivergence ? 'known SDK divergence' : 'pinned SDK parity'}: ${trace.name}`, () => {
    assert.equal(reference.sdk, 'ahp-1.0.0', 'Generate the actual pinned SDK oracle before running frontend tests');
    const expected: Json = reference.traces.find((item: Json) => item.name === trace.name);
    assert.ok(expected, `Missing pinned reference for ${trace.name}`);
    assert.equal(expected.states.length, trace.actions.length + 1);
    let current = chat(trace.initial, trace.initial.resource);
    assert.deepEqual(wire(current), expected.states[0]);
    for (let index = 0; index < trace.actions.length; index++) {
      current = reduceChat(current, trace.actions[index]);
      const actual = wire(current) as Json;
      const sdk = expected.states[index + 1];
      if (trace.knownDivergence) {
        // Do not silently bless SDK/documentation disagreement as conformance.
        assert.notDeepEqual(actual, sdk, trace.knownDivergence.reason);
        assert.deepEqual(actual.queuedMessages.map((m: Json) => m.id), trace.knownDivergence.protocolOrder);
        assert.notDeepEqual(sdk.queuedMessages.map((m: Json) => m.id), trace.knownDivergence.protocolOrder);
        const { queuedMessages: actualQueue, ...actualRest } = actual;
        const { queuedMessages: sdkQueue, ...sdkRest } = sdk;
        assert.ok(actualQueue && sdkQueue);
        assert.deepEqual(actualRest, sdkRest, 'Only the documented partial-order difference is allowed');
      } else {
        assert.deepEqual(actual, sdk, `${trace.name}, action ${index}: ${trace.actions[index].type} (${expected.outcomes[index]})`);
      }
    }
  });
}
