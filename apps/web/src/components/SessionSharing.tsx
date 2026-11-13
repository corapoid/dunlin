import { useEffect, useRef, useState } from 'react';
import { activeReaderIds, readSharing, saveSharing, SharingConflict } from '../lib/sharing';
import type { SharingView } from '../lib/sharing';
export function SessionSharing({ sessionId, userId }: { sessionId: string; userId: string }) {
  const [view, setView] = useState<SharingView | null>(null);
  const [selected, setSelected] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [refresh, setRefresh] = useState(0);
  const [saved, setSaved] = useState(false);
  const active = useRef<AbortController | null>(null);
  useEffect(() => {
    const controller = new AbortController(); active.current = controller;
    setView(null); setError(''); setSaved(false); setSelected([]); setBusy(false);
    readSharing(sessionId, userId, controller.signal).then((value) => { if (!controller.signal.aborted) { setView(value); setSelected(activeReaderIds(value)); } }).catch((cause: unknown) => { if (!controller.signal.aborted) setError(cause instanceof Error ? cause.message : 'Nie udało się odczytać udostępnień.'); });
    return () => { controller.abort(); if (active.current === controller) active.current = null; };
  }, [sessionId, userId, refresh]);
  async function save() {
    const controller = active.current;
    if (!view || !controller || controller.signal.aborted || busy) return;
    setBusy(true); setError(''); setSaved(false);
    try {
      const value = await saveSharing(sessionId, userId, view.revision, selected, controller.signal);
      if (!controller.signal.aborted && active.current === controller) { setView(value); setSelected(activeReaderIds(value)); setSaved(true); }
    } catch (cause) {
      if (!controller.signal.aborted && active.current === controller) {
        if (cause instanceof SharingConflict) { setView(cause.current); setSelected(activeReaderIds(cause.current)); }
        else { setView(null); setSelected([]); }
        setError(cause instanceof Error ? cause.message : 'Nie udało się zapisać udostępnień.');
      }
    } finally { if (!controller.signal.aborted && active.current === controller) setBusy(false); }
  }
  const eligible = new Set(view?.eligible_readers.map((r) => r.user_id));
  const readers = new Map(view?.eligible_readers.map((r) => [r.user_id, { ...r, active: true }]));
  for (const reader of view?.readers ?? []) readers.set(reader.user_id, reader);
  return <section className="ahp-sharing" data-testid="ahp-sharing" aria-label="Udostępnianie sesji">
    <h2>Udostępnianie sesji</h2><p>Wybrane osoby zobaczą cały transkrypt, historię rozmowy oraz parametry i wyniki narzędzi. Otrzymają dostęp tylko do odczytu.</p>
    {error && <p role="alert" className="notice error">{error}</p>}
    {!view && error && <button onClick={() => setRefresh((value) => value + 1)}>Wczytaj ponownie</button>}
    {saved && <p role="status">Zapisano udostępnienie.</p>}
    {!view ? !error && <p role="status">Wczytywanie udostępnień…</p> : <>
      <p data-testid="ahp-sharing-revision">Wersja {view.revision}</p>
      {view.readers.some((reader) => !reader.active) && <p className="notice">Zapisanie listy usunie nieaktywne wpisy; ponowne zaznaczenie nada nowy dostęp. Możesz zaznaczyć wyłącznie aktualnie uprawnionych odbiorców.</p>}
      {view.eligible_readers_truncated && <p className="notice">Pokazano pierwszych 256 dostępnych odbiorców. Istniejące udostępnienia nadal możesz usunąć z listy poniżej.</p>}
      <fieldset disabled={busy}><legend>Odbiorcy</legend>{readers.size === 0 ? <p className="muted">Brak dostępnych odbiorców w tym projekcie.</p> : [...readers.values()].map((reader) => <label key={reader.user_id} className="ahp-sharing-reader"><input type="checkbox" data-testid="ahp-sharing-reader" data-user-id={reader.user_id} checked={selected.includes(reader.user_id)} disabled={!eligible.has(reader.user_id) && !selected.includes(reader.user_id)} onChange={(event) => { setSaved(false); setSelected((ids) => event.target.checked ? [...ids, reader.user_id] : ids.filter((id) => id !== reader.user_id)); }} /><span>{reader.name} {!reader.active && <span className="muted">· dostęp nieaktywny</span>}</span></label>)}</fieldset>
      <button data-testid="ahp-sharing-save" disabled={busy || selected.length > 32} onClick={save}>{busy ? 'Zapisywanie…' : 'Zapisz odbiorców'}</button>
      {selected.length > 32 && <p role="alert">Możesz wybrać najwyżej 32 odbiorców.</p>}
      <details data-testid="ahp-sharing-audit"><summary>Historia udostępniania ({view.audit.length})</summary>{view.audit.length === 0 ? <p className="muted">Brak zmian udostępnienia.</p> : <ul>{view.audit.map((entry) => <li key={entry.revision}>Wersja {entry.revision} · <time dateTime={entry.created_at}>{entry.created_at}</time> · {entry.actor_id}<p>Dodano: {entry.added.join(', ') || '—'}; usunięto: {entry.removed.join(', ') || '—'}; odnowiono: {entry.refreshed.join(', ') || '—'}.</p></li>)}</ul>}</details>
    </>}
  </section>;
}
