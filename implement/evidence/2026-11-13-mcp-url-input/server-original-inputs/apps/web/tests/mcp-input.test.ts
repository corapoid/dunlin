import { test } from 'node:test';
import assert from 'node:assert/strict';
import { formAnswers, mcpInputUrl } from '../src/lib/mcp-input.ts';
test('forms preserve false, zero, empty strings/selections and reviewed required empty arrays without applying defaults', () => {
  const questions = [
    { id: 'text', kind: 'text', required: true, defaultValue: 'never auto-submit' },
    { id: 'number', kind: 'integer' }, { id: 'boolean', kind: 'boolean' },
    { id: 'select', kind: 'single-select' }, { id: 'many', kind: 'multi-select', required: true },
    { id: 'optional', kind: 'text', defaultValue: 'not submitted' },
  ];
  const answers = formAnswers(questions, { number: '0', boolean: false, select: '' });
  assert.deepEqual(JSON.parse(JSON.stringify(answers)), {
    text: { state: 'submitted', value: { kind: 'text', value: '' } },
    number: { state: 'submitted', value: { kind: 'number', value: 0 } },
    boolean: { state: 'submitted', value: { kind: 'boolean', value: false } },
    select: { state: 'submitted', value: { kind: 'selected', value: '' } },
    many: { state: 'submitted', value: { kind: 'selected-many', value: [] } },
  });
  assert.throws(() => formAnswers([{ id: 'n', kind: 'number' }], { n: 'invalid' }));
});
test('provider property names cannot become prototype mutations or inherited form answers', () => {
  const questions = ['__proto__', 'constructor', 'toString'].map(id => ({ id, kind: 'text' }));
  assert.deepEqual(Object.keys(formAnswers(questions, {})), []);
  const values = JSON.parse('{"__proto__":"first","constructor":"second","toString":"third"}');
  const answers = formAnswers(questions, values);
  assert.equal(Object.getPrototypeOf(answers), null);
  assert.deepEqual(Object.keys(answers), ['__proto__', 'constructor', 'toString']);
  assert.equal(answers.__proto__.value.value, 'first');
});


test('URL review preserves the full address and ASCII host without fetching, credentials or parser ambiguity', () => {
  assert.deepEqual(mcpInputUrl('https://review.example.test:8443/connect?step=1#review'), {href:'https://review.example.test:8443/connect?step=1#review',host:'review.example.test:8443',punycode:false});
  assert.equal(mcpInputUrl('https://bücher.example.test/connect')?.host,'xn--bcher-kva.example.test');
  assert.equal(mcpInputUrl('https://bücher.example.test/connect')?.punycode,true);
  for (const value of [null, '', '/relative', 'javascript:alert(1)', 'http://example.test', 'https://user:pass@example.test', 'https://example.test/a b', 'https://example.test/\\other', 'https://example.test/\tpath', 'https://example.test/'+ 'x'.repeat(4096)]) assert.equal(mcpInputUrl(value),null);
});
