import { useEffect, useRef, useState } from 'react';
import { decodeWorkspaces, teamRequest, TeamRequestError } from '../lib/teams';
import type { Workspace } from '../lib/teams';
import {
  readHosts,
  createHost,
  enrollHost,
  changeHost,
  hostHarnesses,
  decodeHostDefinition,
  HostRegistryReadFence,
  retainHostEnrollment,
} from '../lib/hosts';
import type { Host, HostDefinition, HostEnrollment } from '../lib/hosts';
import { HostChannelDiagnostic } from './HostChannelDiagnostic';
import { HostHealthLive } from './HostHealthLive';
const statusLabels = {
  pending: 'Oczekuje na zgłoszenie',
  claimed: 'Zgłoszony przez worker',
  approved: 'Zatwierdzony',
  revoked: 'Odwołany',
};
function Registry({ workspace, revisionKey }: { workspace: string; revisionKey: string }) {
  const [hosts, setHosts] = useState<Host[]>([]),
    [loading, setLoading] = useState(true),
    [available, setAvailable] = useState(false),
    [busy, setBusy] = useState(false),
    [checking, setChecking] = useState(false),
    [error, setError] = useState(''),
    [forbidden, setForbidden] = useState(false),
    [needsReload, setNeedsReload] = useState(false),
    [ticket, setTicket] = useState<HostEnrollment | null>(null),
    [copied, setCopied] = useState(false),
    [message, setMessage] = useState('');
  const [definition, setDefinition] = useState<HostDefinition>({
    name: '',
    node_id: '',
    serve_origin: '',
    capability_policy: { max_parallel: 1, harnesses: [] },
  });
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  const fence = useRef(new HostRegistryReadFence());
  const deferredRead = useRef(false);
  async function revalidate(explicit = false, signal?: AbortSignal) {
    const read = fence.current.beginRead();
    const authorityOnly = read === null;
    const generation = read ?? fence.current.authorityGeneration();
    if (authorityOnly) deferredRead.current = true;
    setChecking(true);
    if (explicit) {
      setLoading(true);
      setError('');
      setTicket(null);
      setCopied(false);
    }
    const current = () => alive.current && !signal?.aborted && fence.current.current(generation);
    try {
      const h = await readHosts(workspace, signal);
      if (current() && !authorityOnly) {
        setHosts(h);
        setAvailable(true);
        setForbidden(false);
        setTicket((previous) => (explicit ? null : retainHostEnrollment(previous, h)));
        if (explicit) {
          setNeedsReload(false);
          setMessage('Wczytano aktualny rejestr. Sprawdź stan przed kolejną operacją.');
        }
      }
    } catch (e) {
      if (current()) {
        if (authorityOnly) fence.current.invalidate();
        setHosts([]);
        setAvailable(false);
        setTicket(null);
        setCopied(false);
        setForbidden(e instanceof TeamRequestError && (e.status === 401 || e.status === 403));
        setError(e instanceof Error ? e.message : 'Nie udało się wczytać hostów.');
      }
    } finally {
      if (current()) {
        setLoading(false);
        setChecking(false);
      }
    }
  }
  useEffect(() => {
    const c = new AbortController();
    void revalidate(false, c.signal);
    return () => c.abort();
  }, [workspace, revisionKey]);
  async function mutate(operation: () => Promise<Host | HostEnrollment>) {
    let generation: number;
    try {
      generation = fence.current.beginMutation();
    } catch {
      return;
    }
    setChecking(false);
    setBusy(true);
    setError('');
    setMessage('');
    setTicket(null);
    setCopied(false);
    try {
      const result = await operation();
      if (alive.current && fence.current.current(generation)) {
        const host = 'host' in result ? result.host : result;
        setHosts((all) => [...all.filter((h) => h.id !== host.id), host]);
        if ('enrollment' in result) setTicket(result);
        setMessage('Zapisano zmianę rejestru. Gotowość wykonawcza pozostaje niezweryfikowana.');
      }
    } catch (e) {
      if (alive.current && fence.current.current(generation)) {
        setNeedsReload(true);
        setError(
          e instanceof TeamRequestError && e.status === 409
            ? 'Host zmienił się w innej karcie. Odśwież rejestr przed ponowieniem.'
            : e instanceof Error
              ? e.message
              : 'Wynik operacji jest nieznany.',
        );
        if (e instanceof TeamRequestError && (e.status === 401 || e.status === 403)) {
          setForbidden(true);
          setAvailable(false);
          setHosts([]);
        }
      }
    } finally {
      fence.current.endMutation();
      if (alive.current) {
        setBusy(false);
        setChecking(false);
        if (deferredRead.current) {
          deferredRead.current = false;
          void revalidate();
        }
      }
    }
  }
  async function copy() {
    if (!ticket) return;
    try {
      await navigator.clipboard.writeText(ticket.enrollment.ticket);
      setCopied(true);
    } catch {
      setError('Nie udało się skopiować biletu. Możesz zaznaczyć go ręcznie.');
    }
  }
  function denyDiagnostics() {
    if (!alive.current) return;
    fence.current.invalidate();
    setHosts([]);
    setTicket(null);
    setCopied(false);
    setAvailable(false);
    setForbidden(true);
    setNeedsReload(true);
    setChecking(false);
    setError('Dostęp do diagnostyki hosta został odrzucony. Dane rejestru i bilet usunięto z widoku.');
  }
  if (loading) return <p role="status">Wczytywanie rejestru hostów…</p>;
  return (
    <section data-testid="host-registry">
      <h2>Rejestr hostów</h2>
      <p className="notice">
        Rejestracja i raport workera nie potwierdzają połączenia Tailscale Serve. Sieć: niezweryfikowana.
        Wykonywanie zadań na tych hostach jest niedostępne.
      </p>
      {error && (
        <p className="notice error" role="alert">
          {error}
        </p>
      )}
      {message && <p role="status">{message}</p>}
      <button data-testid="hosts-refresh" disabled={busy || checking} onClick={() => void revalidate(true)}>
        Odśwież rejestr
      </button>
      {forbidden ? (
        <p data-testid="hosts-forbidden">Brak uprawnień do zarządzania hostami tego zespołu.</p>
      ) : available ? (
        <>
          {needsReload && (
            <p role="alert">
              Operacja nie zostanie automatycznie powtórzona. Wczytaj aktualny rejestr przed następną zmianą;
              utracony bilet wymaga jawnego wydania nowego.
            </p>
          )}
          {ticket && (
            <section data-testid="host-enrollment" className="notice">
              <h3>Jednorazowo pokazany bilet rejestracji</h3>
              <p>Przekaż go wybranemu workerowi poza przeglądarką. Nie jest dowodem gotowości hosta.</p>
              <output data-testid="host-ticket" className="host-ticket">
                {ticket.enrollment.ticket}
              </output>
              <p>
                Wygasa:{' '}
                <time dateTime={new Date(ticket.enrollment.expires_at * 1000).toISOString()}>
                  {new Date(ticket.enrollment.expires_at * 1000).toLocaleString('pl')}
                </time>
              </p>
              <button disabled={checking || busy} data-testid="host-ticket-copy" onClick={() => void copy()}>
                Kopiuj bilet
              </button>
              <button
                onClick={() => {
                  setTicket(null);
                  setCopied(false);
                }}
              >
                Ukryj bilet
              </button>
              {copied && <p role="status">Skopiowano na Twoje żądanie.</p>}
            </section>
          )}
          {!hosts.length && <p data-testid="hosts-empty">Brak zarejestrowanych hostów.</p>}
          <ul className="host-list">
            {hosts.map((host) => (
              <li key={host.id} data-testid="host-row" data-host-id={host.id}>
                <h3>{host.definition.name}</h3>
                <p>
                  {host.definition.node_id} · {host.definition.serve_origin}
                </p>
                <p data-testid="host-status">{statusLabels[host.status]}</p>
                <p>
                  Sieć niezweryfikowana w rejestrze · rewizja {host.revision}
                  {host.drain ? ' · Drenaż włączony' : ''}
                </p>
                <p className="muted">Powiązanie projektu i profil sesji są weryfikowane przy uruchomieniu.</p>
                {host.capabilities && (
                  <details>
                    <summary>Zadeklarowane możliwości workera</summary>
                    <p>Raport nie potwierdza weryfikacji sieci ani wykonania testu połączenia.</p>
                    <p>
                      {host.capabilities.os} / {host.capabilities.architecture} · wersja{' '}
                      {host.capabilities.version} · sandbox {host.capabilities.sandbox}
                    </p>
                    <p>
                      Harnessy: {host.capabilities.harnesses.join(', ')} · równoległość{' '}
                      {host.capabilities.max_parallel}
                    </p>
                  </details>
                )}
                <div className="host-actions">
                  <button
                    data-testid="host-enroll"
                    disabled={busy || checking || needsReload}
                    onClick={() => void mutate(() => enrollHost(workspace, host))}
                  >
                    Wydaj nowy bilet rejestracji
                  </button>
                  <button
                    data-testid="host-approve"
                    disabled={busy || checking || needsReload || host.status !== 'claimed'}
                    onClick={() => void mutate(() => changeHost(workspace, host, 'approve'))}
                  >
                    Zatwierdź zgłoszony host
                  </button>
                  <button
                    data-testid="host-drain"
                    disabled={busy || checking || needsReload || host.status === 'revoked'}
                    onClick={() => void mutate(() => changeHost(workspace, host, 'drain', !host.drain))}
                  >
                    {host.drain ? 'Wyłącz drenaż' : 'Włącz drenaż'}
                  </button>
                  <button
                    data-testid="host-revoke"
                    disabled={busy || checking || needsReload || host.status === 'revoked'}
                    onClick={() => void mutate(() => changeHost(workspace, host, 'revoke'))}
                  >
                    Odwołaj host
                  </button>
                </div>
                {!checking && !busy && !needsReload && (
                  <>
                    <HostHealthLive
                      key={`health:${revisionKey}:${host.id}:${host.revision}:${host.enrollment_epoch}:${host.credential_revision}`}
                      workspace={workspace} host={host.id} onForbidden={denyDiagnostics}
                    />
                  <HostChannelDiagnostic
                    key={`${revisionKey}:${host.id}:${host.revision}:${host.enrollment_epoch}:${host.credential_revision}`}
                    workspace={workspace}
                    host={host.id}
                    onForbidden={denyDiagnostics}
                  />
                  </>
                )}
              </li>
            ))}
          </ul>
          <form
            className="agent-form"
            data-testid="host-create"
            onSubmit={(e) => {
              e.preventDefault();
              try {
                const clean = decodeHostDefinition(definition);
                void mutate(() => createHost(workspace, clean));
              } catch (cause) {
                setError(cause instanceof Error ? cause.message : 'Niepoprawna definicja.');
              }
            }}
          >
            <fieldset disabled={busy || checking || needsReload}>
              <legend>Zarejestruj oczekiwany host</legend>
              <label>
                Nazwa hosta
                <input
                  required
                  data-testid="host-name"
                  value={definition.name}
                  onChange={(e) => setDefinition({ ...definition, name: e.target.value })}
                />
              </label>
              <label>
                Oczekiwany stabilny identyfikator węzła
                <input
                  required
                  data-testid="host-node"
                  value={definition.node_id}
                  onChange={(e) => setDefinition({ ...definition, node_id: e.target.value })}
                />
              </label>
              <label>
                Origin HTTPS Tailscale Serve
                <input
                  required
                  type="url"
                  data-testid="host-origin"
                  placeholder="https://worker.tail123.ts.net"
                  value={definition.serve_origin}
                  onChange={(e) => setDefinition({ ...definition, serve_origin: e.target.value })}
                />
              </label>
              <label>
                Maksymalna równoległość
                <input
                  required
                  type="number"
                  min={1}
                  max={32}
                  value={definition.capability_policy.max_parallel}
                  onChange={(e) =>
                    setDefinition({
                      ...definition,
                      capability_policy: {
                        ...definition.capability_policy,
                        max_parallel: Number(e.target.value),
                      },
                    })
                  }
                />
              </label>
              <fieldset>
                <legend>Dozwolone harnessy</legend>
                {hostHarnesses.map((h) => (
                  <label key={h}>
                    <input
                      type="checkbox"
                      checked={definition.capability_policy.harnesses.includes(h)}
                      onChange={(e) =>
                        setDefinition({
                          ...definition,
                          capability_policy: {
                            ...definition.capability_policy,
                            harnesses: e.target.checked
                              ? [...definition.capability_policy.harnesses, h]
                              : definition.capability_policy.harnesses.filter((v) => v !== h),
                          },
                        })
                      }
                    />
                    {h}
                  </label>
                ))}
              </fieldset>
              <button className="primary" data-testid="host-create-submit">
                Zarejestruj host i wydaj bilet
              </button>
            </fieldset>
          </form>
        </>
      ) : null}
    </section>
  );
}
export function HostRegistry({ userId, revisionKey }: { userId: string; revisionKey: string }) {
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]),
    [workspace, setWorkspace] = useState(''),
    [loading, setLoading] = useState(true),
    [error, setError] = useState('');
  useEffect(() => {
    const c = new AbortController();
    teamRequest('/v1/web/workspaces', c.signal)
      .then((v) => {
        if (!c.signal.aborted) setWorkspaces(decodeWorkspaces(v, userId));
      })
      .catch((e) => {
        if (!c.signal.aborted) setError(e.message);
      })
      .finally(() => {
        if (!c.signal.aborted) setLoading(false);
      });
    return () => c.abort();
  }, [userId]);
  return (
    <div className="agent-workspace">
      <h1>Hosty</h1>
      <p>
        Rejestruj oczekiwane węzły i zatwierdzaj ich zgłoszenia. Zarządzanie wymaga uprawnień w wybranym
        zespole.
      </p>
      {loading && <p role="status">Wczytywanie zespołów…</p>}
      {error && <p role="alert">{error}</p>}
      {!loading && !workspaces.length && !error && <p>Brak dostępnych zespołów.</p>}
      <label>
        Zespół hostów
        <select data-testid="host-workspace" value={workspace} onChange={(e) => setWorkspace(e.target.value)}>
          <option value="">Wybierz zespół</option>
          {workspaces.map((w) => (
            <option key={w.id} value={w.id}>
              {w.name}
            </option>
          ))}
        </select>
      </label>
      {workspace && (
        <Registry key={`${userId}:${workspace}`} workspace={workspace} revisionKey={revisionKey} />
      )}
    </div>
  );
}
