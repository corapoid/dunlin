import { useId, useState } from 'react';
import { formAnswers, mcpInputUrl } from '../lib/mcp-input';
import type { Json } from '../lib/ahp';
import type { AhpObserver } from '../lib/ahp-live';

export function McpInput({ part, allowed, disabled, observer }: { part: Json; allowed: boolean; disabled: boolean; observer: AhpObserver | null }) {
  const prefix = useId();
  const [values, setValues] = useState<Json>(() => Object.create(null));
  const request = part.request ?? {};
  const questions: Json[] = Array.isArray(request.questions) ? request.questions : [];
  const urlMode = request.url !== undefined && request.url !== null;
  const target = urlMode ? mcpInputUrl(request.url) : null;
  const supported = typeof request.id === 'string' && request.id.startsWith('dunlin:mcp-input:') && (urlMode ? !!target && request.questions === undefined && request.answers === undefined : questions.length <= 32 && questions.every(q => ['text', 'number', 'integer', 'boolean', 'single-select', 'multi-select'].includes(q.kind)));
  function set(id: string, value: unknown) { setValues(previous => ({ ...previous, [id]: value })); }
  function complete(response: 'accept' | 'decline' | 'cancel') {
    const answers = response === 'accept' && !urlMode ? formAnswers(questions, values) : undefined;
    observer?.completeInput(request.id, response, response === 'accept' ? answers : undefined);
  }
  if (part.response) return <details className="ahp-detail" data-testid="ahp-input-resolved"><summary>{urlMode ? 'Adres MCP' : 'Formularz MCP'} · {part.response === 'accept' ? 'wysłano odpowiedź' : part.response === 'decline' ? 'odmowa' : 'anulowano'}</summary><p className="ahp-text">{request.message}</p>{request.answers && <pre>{JSON.stringify(request.answers, null, 2)}</pre>}</details>;
  return <form className="ahp-detail mcp-input" data-testid="ahp-input" data-request-id={request.id} onSubmit={event => { event.preventDefault(); complete('accept'); }}>
    <h4>{urlMode ? 'Strona wskazana przez narzędzie MCP' : 'Dodatkowe dane dla narzędzia MCP'}</h4><p className="ahp-text">{request.message}</p>
    {urlMode ? target && <div data-testid="ahp-input-url-review">
      <p>Domena docelowa: <strong data-testid="ahp-input-url-host">{target.host}</strong></p>
      <p className="ahp-text" data-testid="ahp-input-url-address">{target.address}</p>
      {target.punycode && <p>Domena zawiera zapis punycode. Sprawdź jej właściciela przed otwarciem.</p>}
      <p>Otwórz stronę, jeśli zgadzasz się na tę interakcję. Dunlin nie odczytuje jej treści ani wpisanych danych. Gdy zechcesz wznowić narzędzie, wybierz „Kontynuuj”.</p>
      {supported && allowed && !disabled && <a data-testid="ahp-input-url-open" href={target.href} target="_blank" rel="noopener noreferrer" referrerPolicy="no-referrer">Otwórz adres w nowej karcie</a>}
    </div> : <p>Sprawdź dane przed wysłaniem. Nie wpisuj haseł ani tokenów dostępu.</p>}
    {supported ? !urlMode && questions.map((q, index) => {
      const id = `${prefix}-${index}`;
      const value = Object.hasOwn(values, q.id) ? values[q.id] : undefined;
      return <div key={q.id} data-question-id={q.id}>
        <label htmlFor={id}>{q.title ?? q.id}{q.required ? ' *' : ''}</label>
        {q.message && q.message !== q.title && <p className="muted">{q.message}</p>}
        {q.kind === 'text' && <input id={id} type={q.format === 'email' ? 'email' : q.format === 'date' ? 'date' : 'text'} value={value ?? ''} required={q.required && q.min > 0} minLength={q.min} maxLength={Math.min(q.max ?? 8192, 8192)} disabled={disabled || !allowed} onChange={e => set(q.id, e.target.value)} />}
        {['number', 'integer'].includes(q.kind) && <input id={id} type="number" step={q.kind === 'integer' ? 1 : 'any'} min={q.min} max={q.max} value={value ?? ''} required={q.required} disabled={disabled || !allowed} onChange={e => set(q.id, e.target.value)} />}
        {q.kind === 'boolean' && <select id={id} value={value === undefined ? '' : String(value)} required={q.required} disabled={disabled || !allowed} onChange={e => set(q.id, e.target.value === '' ? undefined : e.target.value === 'true')}><option value="">Wybierz…</option><option value="true">Tak</option><option value="false">Nie</option></select>}
        {q.kind === 'single-select' && <select id={id} value={value === undefined ? '' : String(q.options.findIndex((o: Json) => o.id === value))} required={q.required} disabled={disabled || !allowed} onChange={e => set(q.id, e.target.value === '' ? undefined : q.options[Number(e.target.value)].id)}><option value="">Wybierz…</option>{q.options.map((o: Json, i: number) => <option key={i} value={i}>{o.label || '(pusta wartość)'}{o.recommended ? ' (propozycja)' : ''}</option>)}</select>}
        {q.kind === 'multi-select' && <select id={id} multiple value={(value ?? []).map((v: string) => String(q.options.findIndex((o: Json) => o.id === v)))} required={q.min > 0} disabled={disabled || !allowed} onChange={e => set(q.id, Array.from(e.target.selectedOptions, o => q.options[Number(o.value)].id))}>{q.options.map((o: Json, i: number) => <option key={i} value={i}>{o.label || '(pusta wartość)'}{o.recommended ? ' (propozycja)' : ''}</option>)}</select>}
        {q.defaultValue !== undefined && <p className="muted">Proponowana wartość: {JSON.stringify(q.defaultValue)}. Wprowadź ją, jeśli chcesz jej użyć.</p>}
      </div>;
    }) : <p role="alert">Ten typ formularza nie jest obsługiwany.</p>}
    {allowed && supported && <div><button data-testid="ahp-input-accept" className="primary" disabled={disabled}>{urlMode ? 'Kontynuuj' : 'Wyślij odpowiedź'}</button><button data-testid="ahp-input-decline" type="button" disabled={disabled} onClick={() => complete('decline')}>Odmów</button><button data-testid="ahp-input-cancel" type="button" disabled={disabled} onClick={() => complete('cancel')}>{urlMode ? 'Anuluj interakcję' : 'Anuluj formularz'}</button></div>}
  </form>;
}
