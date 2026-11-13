import type { Json } from './ahp.ts';
/** Defaults are display metadata. Only explicit answers and reviewed empty
 * required strings/arrays become content; 0, false and enum "" are preserved. */
export function formAnswers(questions: Json[], values: Json): Json {
  const answers: Json = Object.create(null);
  for (const q of questions) {
    let value = Object.hasOwn(values, q.id) ? values[q.id] : undefined;
    if (value === undefined && q.required && q.kind === 'text') value = '';
    if (value === undefined && q.required && q.kind === 'multi-select') value = [];
    if (value === undefined || ['number', 'integer'].includes(q.kind) && value === '') continue;
    const kind = q.kind === 'text' ? 'text' : ['number', 'integer'].includes(q.kind) ? 'number' : q.kind === 'boolean' ? 'boolean' : q.kind === 'single-select' ? 'selected' : q.kind === 'multi-select' ? 'selected-many' : undefined;
    if (!kind) throw new Error('Nieobsługiwany typ pytania.');
    const typed = kind === 'number' ? Number(value) : value;
    if (kind === 'number' && !Number.isFinite(typed)) throw new Error('Wprowadź poprawną liczbę.');
    answers[q.id] = { state: 'submitted', value: { kind, value: typed } };
  }
  return answers;
}


/** Display-only link. No network request or navigation occurs in this codec. */
export function mcpInputUrl(value: unknown): { href: string; host: string; punycode: boolean } | null {
  if (typeof value !== 'string' || value.length > 4096 || /[\x00-\x20\x7f\\]/u.test(value)) return null;
  try {
    const url = new URL(value);
    if (url.protocol !== 'https:' || !url.hostname || url.username || url.password) return null;
    return { href: value, host: url.host, punycode: url.hostname.split('.').some(label => label.startsWith('xn--')) };
  } catch { return null; }
}
