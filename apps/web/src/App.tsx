import { useCallback, useEffect, useRef, useState } from 'react';
import type { FormEvent } from 'react';
import { Frame, FrameDescription, FrameHeader, FramePanel, FrameTitle } from './components/reui/frame';
import type { CatalogSnapshot } from './lib/catalog';
import { CatalogClient } from './lib/live';
import type { LiveStatus } from './lib/live';
import { validateEndpoint } from './lib/transport';
import { TeamSettings } from './components/TeamSettings';
import { AhpSessions } from './components/AhpSessions';
import { OidcLogin } from './components/OidcLogin';
import { AgentBuilder } from './components/AgentBuilder';
import { SessionCreation } from './components/SessionCreation';
import { SessionCreationClient } from './lib/session-create';
import { McpConnections } from './components/McpConnections';
import { consumeMcpReturn } from './lib/mcp-oauth';
import { HostRegistry } from './components/HostRegistry';

type Identity = { user: { id: string; name: string; role: string }; device_id: string; capabilities?: { ahp_observer?: boolean }; ahp_endpoint?: string };
type AuthState = 'loading' | 'signed_out' | 'signed_in' | 'error';

const statusLabels: Record<LiveStatus, string> = {
  connecting: 'Łączenie…', live: 'Na żywo', reconnecting: 'Ponowne łączenie…',
  offline: 'Offline', error: 'Błąd synchronizacji', authorization_required: 'Sesja wygasła',
};

async function identity(response: Response): Promise<Identity> {
  const value: unknown = await response.json();
  if (!value || typeof value !== 'object' || !('user' in value) || !('device_id' in value)
    || typeof value.device_id !== 'string' || !value.user || typeof value.user !== 'object'
    || !('id' in value.user) || !('name' in value.user) || !('role' in value.user)
    || typeof value.user.id !== 'string' || typeof value.user.name !== 'string' || typeof value.user.role !== 'string') {
    throw new Error('Niepoprawna odpowiedź serwera.');
  }
  return value as Identity;
}

export function App() {
  const [auth, setAuth] = useState<AuthState>('loading');
  const [user, setUser] = useState<Identity | null>(null);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [catalog, setCatalog] = useState<CatalogSnapshot | null>(null);
  const [status, setStatus] = useState<LiveStatus>('connecting');
  const [project, setProject] = useState<string | null>(null);
  const [search, setSearch] = useState('');
  const [retry, setRetry] = useState(0);
  const [view, setView] = useState<'projects' | 'teams' | 'sessions' | 'agents' | 'hosts' | 'mcp'>('projects');
  const [mcpWorkspace,setMcpWorkspace]=useState<string>();
  useEffect(()=>{if(user){const workspace=consumeMcpReturn(user.user.id);setMcpWorkspace(workspace);if(workspace){setView('mcp');}}},[user?.user.id]);
  const creation = useRef<{identity:string;client:SessionCreationClient}|null>(null);
  const [,setCreationVersion] = useState(0);
  const creationChanged = useCallback(() => setCreationVersion(n=>n+1),[]);
  const browserIdentity = user ? `${user.user.id}:${user.device_id}` : '';
  if (user && creation.current?.identity !== browserIdentity) creation.current = {identity:browserIdentity,client:new SessionCreationClient({userId:user.user.id,deviceId:user.device_id})};
  const onExpired = useCallback(() => setStatus('authorization_required'), []);
  const recheckIdentity=useCallback(()=>{setCatalog(null);setUser(null);setAuth('loading');setRetry(n=>n+1);},[]);
  const live = useRef<CatalogClient | null>(null);

  useEffect(() => {
    const controller = new AbortController();
    try {
      validateEndpoint(window.location.origin);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Nieobsługiwany adres.');
      setAuth('error');
      return;
    }
    fetch('/v1/web/session', { credentials: 'same-origin', cache: 'no-store', signal: controller.signal })
      .then(async (response) => {
        if (controller.signal.aborted) return;
        if (response.status === 401) { setAuth('signed_out'); return; }
        if (!response.ok) throw new Error('Nie udało się zweryfikować sesji lub uprawnień.');
        const current = await identity(response);
        if (!controller.signal.aborted) { setUser(current); setAuth('signed_in'); }
      })
      .catch((cause: unknown) => {
        if (!controller.signal.aborted) {
          setError(cause instanceof Error ? cause.message : 'Serwer jest niedostępny.');
          setAuth('error');
        }
      });
    return () => controller.abort();
  }, [retry]);

  useEffect(() => {
    if (auth !== 'signed_in' || !user) return;
    const expectedUser = user.user.id;
    const client = new CatalogClient(window.location.origin, (snapshot) => {
      if (snapshot.user.id !== expectedUser) {
        client.stop();
        setCatalog(null);
        setProject(null);
        setAuth('loading');
        setRetry((value) => value + 1);
        return;
      }
      setCatalog(snapshot);
    }, setStatus);
    live.current = client;
    client.start();
    return () => { client.stop(); if (live.current === client) live.current = null; };
  }, [auth, retry, user?.user.id]);

  useEffect(() => { live.current?.selectProject(project); }, [project, auth, retry]);

  useEffect(() => {
    if (status === 'error') {
      setCatalog(null);
    }
    if (status === 'authorization_required') {
      setCatalog(null);
      setProject(null);
      setUser(null);
      setAuth('signed_out');
      setError('Sesja wygasła lub dostęp został odwołany. Sparuj przeglądarkę ponownie.');
    }
  }, [status]);

  async function pair(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = event.currentTarget;
    const values = new FormData(form);
    const code = String(values.get('code') ?? '').trim();
    setBusy(true);
    setError('');
    try {
      const response = await fetch('/v1/web/pair', {
        method: 'POST', credentials: 'same-origin', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ code, device_name: String(values.get('device_name') ?? 'Dunlin Web') }),
      });
      if (!response.ok) throw new Error(response.status === 429 ? 'Zbyt wiele prób. Spróbuj później.' : 'Kod jest nieprawidłowy, wygasł lub brakuje uprawnień.');
      await identity(response);
      form.reset();
      setAuth('loading');
      setRetry((value) => value + 1);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Parowanie nie powiodło się.');
    } finally { setBusy(false); }
  }

  async function logout() {
    setBusy(true);
    setError('');
    try {
      const response = await fetch('/v1/web/logout', { method: 'POST', credentials: 'same-origin' });
      if (!response.ok && response.status !== 401) throw new Error('Wylogowanie nie powiodło się.');
      live.current?.stop();
      setCatalog(null); setProject(null); setUser(null); setAuth('signed_out');
    } catch (cause) { setError(cause instanceof Error ? cause.message : 'Błąd wylogowania.'); }
    finally { setBusy(false); }
  }

  function reconnect() {
    setCatalog(null); setProject(null); setAuth('loading'); setError(''); setRetry((value) => value + 1);
  }

  if (auth !== 'signed_in') return (
    <main className="onboarding">
      <div className="brand"><span className="brand-mark" aria-hidden="true">D</span> Dunlin <span className="preview">WEB · PREVIEW</span></div>
      <Frame className="pairing-frame">
        <FrameHeader><FrameTitle>Twoje projekty. Twoja sieć.</FrameTitle><FrameDescription>Połącz przeglądarkę z prywatnym serwerem Dunlin.</FrameDescription></FrameHeader>
        <FramePanel>
          {auth === 'loading' ? <p role="status">Sprawdzanie sesji…</p> : auth === 'error' ? <>
            <p role="alert">{error}</p><button onClick={reconnect}>Spróbuj ponownie</button>
          </> : <form onSubmit={pair} className="pairing-form">
            <label htmlFor="pair-code">Jednorazowy kod parowania</label>
            <input id="pair-code" name="code" type="password" autoComplete="off" required minLength={8} maxLength={64} autoCapitalize="characters" aria-describedby="pair-help" />
            <p id="pair-help" className="muted">Administrator generuje kod poleceniem <code>dunlin-relay pair NAZWA_UŻYTKOWNIKA</code>. Kod jest ważny maksymalnie 15 minut.</p>
            <label htmlFor="device-name">Nazwa przeglądarki</label>
            <input id="device-name" name="device_name" defaultValue="Dunlin Web" required maxLength={120} autoComplete="off" />
            {error && <p role="alert" className="error">{error}</p>}
            <button className="primary" disabled={busy}>{busy ? 'Parowanie…' : 'Połącz bezpiecznie'}</button>
          </form>}
          {auth === 'signed_out' && <OidcLogin />}
        </FramePanel>
      </Frame>
      <p className="muted">Tailscale HTTPS · lokalnie loopback · bez chmury Dunlin</p>
    </main>
  );

  const projects = catalog?.projects.filter((item) => item.name.toLocaleLowerCase().includes(search.toLocaleLowerCase())) ?? [];
  const selected = catalog?.projects.find((item) => item.id === project);
  const currentSessions = catalog?.project_id === project ? catalog.sessions : null;
  return (
    <div className="workspace-shell">
      <a className="skip-link" href="#workspace-content">Przejdź do treści</a>
      <aside className="sidebar">
        <div className="brand"><span className="brand-mark" aria-hidden="true">D</span> Dunlin</div>
        <div className="server-card"><span className="eyebrow">PRYWATNY WORKSPACE</span><strong>{catalog?.server_name ?? 'Dunlin Server'}</strong><span className="muted">{window.location.host}</span></div>
        <nav aria-label="Nawigacja główna">
          <button aria-current={view === 'projects' ? 'page' : undefined} className={view === 'projects' ? 'nav-current' : ''} onClick={() => { setProject(null); setView('projects'); }}>Projekty <span>{catalog?.projects.length ?? '—'}</span></button>
          <button aria-current={view === 'teams' ? 'page' : undefined} className={view === 'teams' ? 'nav-current' : ''} onClick={() => { setProject(null); setView('teams'); }}>Zespół</button>
          <button aria-current={view === 'agents' ? 'page' : undefined} className={view === 'agents' ? 'nav-current' : ''} onClick={() => { setProject(null); setView('agents'); }}>Agenci</button>
          <button aria-current={view === 'mcp' ? 'page' : undefined} className={view === 'mcp' ? 'nav-current' : ''} onClick={() => { setProject(null); setView('mcp'); }}>Połączenia MCP</button>
          <button aria-current={view === 'hosts' ? 'page' : undefined} className={view === 'hosts' ? 'nav-current' : ''} onClick={() => { setProject(null); setView('hosts'); }}>Hosty</button>
          {user?.capabilities?.ahp_observer === true && user.ahp_endpoint === '/v1/web/ahp' && <button aria-current={view === 'sessions' ? 'page' : undefined} className={view === 'sessions' ? 'nav-current' : ''} onClick={() => { setProject(null); setView('sessions'); }}>Sesje</button>}
        </nav>
        <div className="sidebar-roadmap"><span className="eyebrow">W BUDOWIE</span><p>Workflow · Linear</p><p className="muted">Katalog projektów jest aktualizowany na żywo. Obserwacja rozmów wymaga włączenia prywatnego runtime przez administratora.</p></div>
        <div className="profile"><strong>{user?.user.name}</strong><span className="muted">{user?.user.role}</span><button onClick={logout} disabled={busy}>Wyloguj</button></div>
      </aside>
      <main id="workspace-content" className="workspace-content">
        <header className="topbar"><span>Workspace <span aria-hidden="true">/</span> {view === 'teams' ? 'Zespół' : view === 'sessions' ? 'Sesje' : view === 'agents' ? 'Agenci' : view === 'hosts' ? 'Hosty' : view === 'mcp' ? 'Połączenia MCP' : 'Projekty'}</span><span role="status" className={`live-status ${status === 'live' ? 'is-live' : ''}`}><span aria-hidden="true" />{statusLabels[status]}</span></header>
        <section className="workspace-body">
          {view === 'mcp' && user ? <McpConnections key={browserIdentity} userId={user.user.id} onExpired={recheckIdentity} initialWorkspace={mcpWorkspace} revisionKey={`${catalog?.stream_id??'pending'}:${catalog?.workspace_revision??0}`}/> : view === 'hosts' && user ? <HostRegistry key={browserIdentity} userId={user.user.id} revisionKey={`${catalog?.stream_id??'pending'}:${catalog?.workspace_revision??0}`}/> : view === 'agents' && user ? <AgentBuilder key={browserIdentity} userId={user.user.id}/> : view === 'sessions' && user?.capabilities?.ahp_observer === true && user.ahp_endpoint === '/v1/web/ahp' ? <><SessionCreation key={`create:${browserIdentity}`} userId={user.user.id} client={creation.current!.client} onChanged={creationChanged}/><AhpSessions key={browserIdentity} userId={user.user.id} /></> : view === 'teams' ? catalog && user ? <TeamSettings userId={user.user.id} canCreate={user.user.role === 'admin'} revisionKey={`${catalog.stream_id}:${catalog.workspace_revision ?? 0}`} onExpired={onExpired} /> : <p role="status">Oczekiwanie na synchronizację…</p> : <>
          <div className="page-heading"><div><span className="eyebrow">PRZESTRZEŃ PRACY</span><h1>{selected?.name ?? 'Projekty'}</h1><p className="muted">{project ? 'Twoje sesje w wybranym projekcie.' : 'Projekty udostępnione Ci na tym serwerze.'}</p></div><span className="preview">KATALOG LIVE · V1</span></div>
          {error && <p role="alert" className="notice error">{error}</p>}
          {status !== 'live' && <div className="notice"><p>{catalog ? 'Wyświetlane dane mogą być nieaktualne.' : 'Oczekiwanie na autorytatywny snapshot serwera.'}</p>{['offline', 'error'].includes(status) && <button onClick={reconnect}>Połącz ponownie</button>}</div>}
          {project ? <>
            <button className="back-button" onClick={() => setProject(null)}>← Wszystkie projekty</button>
            <Frame><FrameHeader><FrameTitle>Sesje</FrameTitle><FrameDescription>Tylko sesje należące do Twojego konta. Bez zmiany współdzielenia.</FrameDescription></FrameHeader><FramePanel>
              {currentSessions === null ? <p role="status">Ładowanie sesji…</p> : currentSessions.length === 0 ? <div className="empty"><h2>Brak sesji w tym projekcie</h2><p className="muted">Sesje utworzone w podłączonym kliencie pojawią się tutaj automatycznie.</p></div> : <ul className="session-list">{currentSessions.map((session) => <li key={session.id}><div><strong>{session.title || 'Bez tytułu'}</strong><span className="muted">{session.id}</span></div><time dateTime={new Date(session.updated_at * 1000).toISOString()}>{new Date(session.updated_at * 1000).toLocaleString('pl')}</time></li>)}</ul>}
            </FramePanel></Frame>
          </> : <>
            <div className="project-toolbar"><label htmlFor="project-search" className="sr-only">Szukaj projektu</label><input id="project-search" type="search" value={search} onChange={(event) => setSearch(event.target.value)} placeholder="Szukaj projektu…" /><span className="muted">{catalog ? `${projects.length} projektów` : 'Synchronizacja…'}</span></div>
            {catalog && projects.length === 0 ? <Frame><FramePanel><div className="empty"><h2>{search ? 'Brak pasujących projektów' : 'Nie masz jeszcze udostępnionych projektów'}</h2><p className="muted">{search ? 'Zmień wyszukiwaną nazwę.' : 'Poproś administratora o nadanie dostępu. Nie tworzymy przykładowych danych.'}</p></div></FramePanel></Frame> : <div className="project-grid">{projects.map((item) => <Frame key={item.id}><FramePanel><button className="project-card" onClick={() => setProject(item.id)} disabled={status !== 'live'}><span className="project-monogram" aria-hidden="true">{item.name.slice(0, 1).toUpperCase()}</span><strong>{item.name}</strong><span className="muted">Otwórz katalog sesji <span aria-hidden="true">↗</span></span></button></FramePanel></Frame>)}</div>}
          </>}
          </>}
          <footer className="workspace-footer">Dane pozostają na wybranym serwerze. Połączenia między komputerami wyłącznie przez prywatny Tailscale.</footer>
        </section>
      </main>
    </div>
  );
}
