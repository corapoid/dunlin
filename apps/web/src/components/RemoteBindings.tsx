import { useEffect, useRef, useState } from 'react';
import { RemoteBindingsController } from '../lib/remote-bindings';
import type { RemoteBindingInput, RemoteBindingsState } from '../lib/remote-bindings';
import { readHosts } from '../lib/hosts';
import type { Host } from '../lib/hosts';
import { TeamRequestError } from '../lib/teams';
import { RemoteBindingAccess } from './RemoteBindingAccess';
function observedTime(value: number): string {
  const date = new Date(value);
  return Number.isFinite(date.getTime()) ? date.toLocaleString('pl') : `${value} ms od epoki Unix`;
}
export function RemoteBindings({
  workspace,
  project,
  revisionKey,
  userId,
}: {
  workspace: string;
  project: string;
  revisionKey: string;
  userId: string;
}) {
  const [state, setState] = useState<RemoteBindingsState>({
      status: 'loading',
      bindings: [],
      error: '',
      needsReload: false,
    }),
    [hosts, setHosts] = useState<Host[]>([]),
    [selected, setSelected] = useState(''),
    [baseline, setBaseline] = useState<number | null>(null),
    [accessEpoch, setAccessEpoch] = useState(0),
    [input, setInput] = useState<RemoteBindingInput>({
      host_id: '',
      export_id: '',
      expected_export_revision: 0,
    }),
    [error, setError] = useState(''),
    [message, setMessage] = useState(''),
    [refresh, setRefresh] = useState(0),
    [hostsLoading, setHostsLoading] = useState(true);
  const client = useRef<RemoteBindingsController | null>(null),
    alive = useRef(true),
    lastRefresh = useRef(0);
  useEffect(() => {
    alive.current = true;
    const controller = new RemoteBindingsController(workspace, project, setState);
    client.current = controller;
    return () => {
      alive.current = false;
      controller.stop();
      if (client.current === controller) client.current = null;
    };
  }, [workspace, project]);
  useEffect(() => {
    const controller = client.current;
    if (!controller) return;
    const explicit = refresh !== lastRefresh.current;
    lastRefresh.current = refresh;
    void controller.refresh(explicit);
    const abort = new AbortController();
    setHosts([]);
    setHostsLoading(true);
    readHosts(workspace, abort.signal)
      .then((h) => {
        if (!abort.signal.aborted) setHosts(h);
      })
      .catch((cause) => {
        if (!abort.signal.aborted) {
          if (cause instanceof TeamRequestError && [401, 403, 404].includes(cause.status)) controller.deny();
          else setError('Nie udało się odczytać katalogu hostów.');
        }
      })
      .finally(() => {
        if (!abort.signal.aborted) setHostsLoading(false);
      });
    return () => abort.abort();
  }, [workspace, project, revisionKey, refresh]);
  const binding = state.bindings.find((b) => b.id === selected),
    stale = !!binding && baseline !== binding.revision,
    missing = !!selected && !binding && state.status === 'ready',
    busy = state.status === 'loading' || state.status === 'mutating' || hostsLoading;
  function choose(id: string) {
    setAccessEpoch(value => value + 1);
    setSelected(id);
    const saved = state.bindings.find((b) => b.id === id);
    setBaseline(saved?.revision ?? null);
    setInput(
      saved
        ? {
            host_id: saved.host_id,
            export_id: saved.export_id,
            expected_export_revision: saved.export_revision,
          }
        : { host_id: '', export_id: '', expected_export_revision: 0 },
    );
    setError('');
    setMessage('');
  }
  async function save() {
    setError('');
    setMessage('');
    const controller = client.current;
    try {
      const result = await controller?.save(input, selected || null, baseline ?? undefined);
      if (alive.current && client.current === controller && result) {
        setSelected(result.id);
        setBaseline(result.revision);
        setMessage('Zapisano zaobserwowany descriptor exportu. Nie uruchomiono zdalnej pracy.');
      }
    } catch (cause) {
      if (alive.current) setError(cause instanceof Error ? cause.message : 'Niepoprawne dane bindingu.');
    }
  }
  async function revoke() {
    setError('');
    setMessage('');
    const controller = client.current;
    try {
      const result = await controller?.revoke(selected, baseline ?? 0);
      if (alive.current && client.current === controller && result) {
        setBaseline(result.revision);
        setMessage('Odwołano zapisany zdalny binding. Lokalny binding i sesje nie zostały przełączone.');
      }
    } catch (cause) {
      if (alive.current) setError(cause instanceof Error ? cause.message : 'Nie udało się odwołać bindingu.');
    }
  }
  return (
    <section className="remote-bindings" data-testid="remote-bindings">
      <h3>Zdalne bindingi projektu</h3>
      <p>
        Kontroler przypina wskazany export workera po odczycie jego deskryptora przez bieżący kanał. Zapis
        jest ostatnio zaobserwowanymi metadanymi i nie potwierdza aktualnego grantu ani gotowości do
        wykonania.
      </p>
      <p className="notice">
        Zdalne uruchamianie pozostaje niedostępne. Tworzenie sesji nadal korzysta z dotychczasowego lokalnego
        celu; ten panel nie przełącza go i nie wybiera zastępczego hosta.
      </p>
      <button
        data-testid="remote-bindings-refresh"
        disabled={state.status === 'mutating'}
        onClick={() => {
          setRefresh((n) => n + 1);
          setError('');
        }}
      >
        Wczytaj zapisany stan bindingów
      </button>
      {state.status === 'loading' && <p role="status">Wczytywanie bindingów…</p>}
      {(error || state.error) && <p role="alert">{error || state.error}</p>}
      {message && <p role="status">{message}</p>}
      {state.status === 'forbidden' ? (
        <p data-testid="remote-bindings-forbidden">Brak uprawnień do zarządzania hostami i projektem.</p>
      ) : (
        state.status !== 'loading' && (
          <>
            {state.needsReload && (
              <p role="alert">
                Przed następną zmianą wczytaj aktualny zapis. Żądanie nie będzie automatycznie ponawiane.
              </p>
            )}
            {state.bindings.length === 0 && state.status === 'ready' && (
              <p>Brak zapisanych zdalnych bindingów.</p>
            )}
            <label>
              Binding
              <select
                data-testid="remote-binding-select"
                value={selected}
                disabled={busy}
                onChange={(e) => choose(e.target.value)}
              >
                <option value="">Nowy binding</option>
                {state.bindings.map((b) => (
                  <option key={b.id} value={b.id}>
                    {b.id} · {b.active ? 'zapis aktywny' : 'odwołany'} · rewizja {b.revision}
                  </option>
                ))}
              </select>
            </label>
            {missing && (
              <p role="alert">
                Wybranego bindingu nie ma w tym odczycie. Wybierz zapisany binding lub jawnie wybierz nowy.
              </p>
            )}
            {state.bindings.length >= 16 && !selected && (
              <p>Osiągnięto limit 16 zapisanych bindingów projektu.</p>
            )}
            {stale && (
              <p role="alert">
                Rewizja bindingu zmieniła się. Sprawdź zapisane metadane i jawnie wczytaj je do formularza.
              </p>
            )}
            {binding && (
              <button disabled={busy} onClick={() => choose(selected)}>
                Wczytaj wybrany binding do formularza
              </button>
            )}
            {binding && (
              <div data-testid="remote-binding-observed">
                <p>
                  Zapis {binding.active ? 'active' : 'inactive'} · ostatnia obserwacja{' '}
                  {observedTime(binding.observed_at_ms)}
                </p>
                <dl>
                  <dt>Host / enrollment / credential revision</dt>
                  <dd>
                    {binding.host_id} / {binding.host_enrollment_epoch} / {binding.credential_revision}
                  </dd>
                  <dt>Export / rewizja / rodzaj</dt>
                  <dd>
                    {binding.export_id} / {binding.export_revision} / {binding.repo_kind}
                  </dd>
                  <dt>Principal / rewizja grantu / uprawnienia</dt>
                  <dd>
                    {binding.principal.kind}:{binding.principal.id} / {binding.grant_revision} /{' '}
                    {binding.grant_permissions.join(', ') || 'brak'}
                  </dd>
                  <dt>Digest tożsamości checkoutu</dt>
                  <dd>
                    <code>{binding.checkout_identity_digest}</code>
                  </dd>
                  <dt>Digest zatwierdzonych ustawień</dt>
                  <dd>
                    <code>{binding.approved_settings_digest}</code>
                  </dd>
                </dl>
                {!busy && !state.needsReload && (
                  <RemoteBindingAccess
                    key={`${revisionKey}:${binding.id}:${binding.revision}:${binding.active}:${accessEpoch}`}
                    binding={binding} userId={userId} disabled={stale}
                  />
                )}
                <button
                  data-testid="remote-binding-revoke"
                  disabled={busy || stale || state.needsReload || !binding.active}
                  onClick={() => void revoke()}
                >
                  Odwołaj zdalny binding
                </button>
              </div>
            )}
            <form
              className="agent-form"
              onSubmit={(e) => {
                e.preventDefault();
                void save();
              }}
            >
              <fieldset disabled={busy || stale || missing || state.needsReload || state.status !== 'ready'}>
                <legend>{selected ? 'Jawnie zwiąż ponownie' : 'Dodaj zdalny binding'}</legend>
                <label>
                  Host docelowy
                  <select
                    required
                    data-testid="remote-binding-host"
                    value={input.host_id}
                    onChange={(e) => setInput({ ...input, host_id: e.target.value })}
                  >
                    <option value="">Wybierz zatwierdzony host</option>
                    {input.host_id &&
                      !hosts.some((h) => h.id === input.host_id && h.status === 'approved') && (
                        <option value={input.host_id} disabled>
                          Zapisany host niedostępny: {input.host_id}
                        </option>
                      )}
                    {hosts
                      .filter((h) => h.status === 'approved')
                      .map((h) => (
                        <option key={h.id} value={h.id}>
                          {h.definition.name} · {h.id}
                        </option>
                      ))}
                  </select>
                </label>
                <label>
                  Identyfikator exportu od operatora workera
                  <input
                    data-testid="remote-binding-export"
                    required
                    maxLength={128}
                    pattern="[A-Za-z0-9_-]+"
                    value={input.export_id}
                    onChange={(e) => setInput({ ...input, export_id: e.target.value })}
                  />
                </label>
                <label>
                  Oczekiwana rewizja exportu od operatora
                  <input
                    data-testid="remote-binding-export-revision"
                    required
                    type="number"
                    min={1}
                    max={Number.MAX_SAFE_INTEGER}
                    value={input.expected_export_revision || ''}
                    onChange={(e) => setInput({ ...input, expected_export_revision: Number(e.target.value) })}
                  />
                </label>
                <p className="muted">
                  Wybór zatwierdzonego hosta nie potwierdza bieżącego kanału. Podaj nieprzezroczyste ID i
                  rewizję exportu; ścieżki i principal nie są danymi tego formularza.
                </p>
                <button
                  data-testid="remote-binding-save"
                  disabled={
                    (!selected && state.bindings.length >= 16) ||
                    !hosts.some((h) => h.id === input.host_id && h.status === 'approved')
                  }
                >
                  Zapisz wskazany zdalny cel
                </button>
              </fieldset>
            </form>
          </>
        )
      )}
    </section>
  );
}
