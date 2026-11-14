import { useEffect, useState } from 'react';
import { AgentScope } from './AgentScope';
import { readProfiles } from '../lib/agents';
import type { ProfileCatalog } from '../lib/agents';
import { SessionCreationClient } from '../lib/session-create';
import { readRemoteBindings } from '../lib/remote-bindings';
import type { RemoteBinding } from '../lib/remote-bindings';
import { McpOAuthClient, type McpConnection } from '../lib/mcp-oauth';
import { ScopedSecretsClient, isSecretProvider, type SecretGrant } from '../lib/scoped-secrets';
function Profiles({
  userId,
  workspace,
  project,
  client,
  changed,
}: {
  userId: string;
  workspace: string;
  project: string;
  client: SessionCreationClient;
  changed: () => void;
}) {
  const [catalog, setCatalog] = useState<ProfileCatalog | null>(null),
    [selected, setSelected] = useState(''),
    [target, setTarget] = useState('local'),
    [bindings, setBindings] = useState<RemoteBinding[]>([]),
    [bindingError, setBindingError] = useState(''),
    [error, setError] = useState(''),
    [loading, setLoading] = useState(true),
    [busy, setBusy] = useState(false);
  const [connections,setConnections]=useState<McpConnection[]>([]),
    [accounts,setAccounts]=useState<Record<string,string>>({}),
    [connectionError,setConnectionError]=useState(''),
    [connectionsLoading,setConnectionsLoading]=useState(false),
    [reloadConnections,setReloadConnections]=useState(0);
  const [secretGrants,setSecretGrants]=useState<SecretGrant[]>([]);
  const remoteBinding = target === 'local' ? undefined : bindings.find(binding => binding.id === target && binding.active);
  const admissionStatus=client.current?.state.status;
  useEffect(()=>{if(admissionStatus==='accepted')setError('');},[admissionStatus]);
  const selectedProfile=catalog?.profiles.find(p=>p.agent_id===selected);
  const providers=[...new Set(selectedProfile?.definition.mcp?.map(t=>t.provider_id)??[])];
  const mcpAvailable=target==='local'?catalog?.mcp_sessions_supported===true:catalog?.remote_mcp_sessions_supported===true;
  useEffect(()=>{
    const controller=new AbortController();
    setConnections([]);setSecretGrants([]);setAccounts({});setConnectionError('');setConnectionsLoading(false);
    if(mcpAvailable&&catalog?.profiles.find(p=>p.agent_id===selected)?.definition.mcp?.length){
      setConnectionsLoading(true);
      const profile=catalog.profiles.find(p=>p.agent_id===selected)!;
      Promise.all([
        providers.some(p=>!isSecretProvider(p))?new McpOAuthClient(workspace,userId,window.location.origin).connections(controller.signal):Promise.resolve([]),
        providers.some(isSecretProvider)&&(target==='local'||remoteBinding)?new ScopedSecretsClient(workspace,project,target==='local'?'local':remoteBinding!.host_id,userId).available({agent_id:profile.agent_id,version:profile.version,digest:profile.digest},controller.signal):Promise.resolve({bindings:[],execution_supported:false}),
      ]).then(([rows,secrets])=>{if(!controller.signal.aborted){setConnections(rows);setSecretGrants(secrets.execution_supported?secrets.bindings:[]);}})
        .catch(error=>{if(!controller.signal.aborted)setConnectionError(error instanceof Error?error.message:'Nie udało się wczytać połączeń MCP.');})
        .finally(()=>{if(!controller.signal.aborted)setConnectionsLoading(false);});
    }
    return ()=>controller.abort();
  },[workspace,project,userId,selected,target,catalog,mcpAvailable,reloadConnections,remoteBinding?.host_id]);
  useEffect(() => {
    const c = new AbortController();
    readProfiles(workspace, project, c.signal)
      .then(async (v) => {
        if (c.signal.aborted) return;
        setCatalog(v);
        if (v.remote_sessions_supported) {
          try {
            const bindings = await readRemoteBindings(workspace, project, c.signal);
            if (!c.signal.aborted) setBindings(bindings);
          } catch (error) {
            if (!c.signal.aborted) setBindingError(error instanceof Error ? error.message : 'Nie udało się wczytać zdalnych celów.');
          }
        }
      })
      .catch((e) => {
        if (!c.signal.aborted) setError(e.message);
      })
      .finally(() => {
        if (!c.signal.aborted) setLoading(false);
      });
    return () => c.abort();
  }, [workspace, project]);
  async function create() {
    const version = catalog?.profiles.find((p) => p.agent_id === selected);
    if (!version || !supported) return;
    setBusy(true);
    setError('');
    try {
      const intent = client.begin(workspace, project, {
        profile: { agent_id: version.agent_id, version: version.version, digest: version.digest },
        ...(providers.length?{mcp_connections:providers.map(provider=>{
          if(isSecretProvider(provider)) {
            const grant=secretGrants.find(g=>g.provider_id===provider&&g.id===accounts[provider]);
            if((target!=='local'&&!remoteBinding)||!grant)throw new Error('Wybierz aktualny grant poświadczenia dla tego hosta i profilu.');
            return {provider_id:provider,connection_id:grant.id,revision:grant.revision};
          }
          const connection=connections.find(c=>c.provider_id===provider&&c.id===accounts[provider]&&c.status==='connected');
          if(!connection)throw new Error('Wybierz aktualne połączenie dla każdego dostawcy MCP.');
          return {provider_id:provider,connection_id:connection.id,revision:connection.revision};
        })}:{}),
        ...(remoteBinding ? {
          target: { kind: 'remote' as const, binding: { id: remoteBinding.id, revision: remoteBinding.revision } },
        } : {}),
      });
      changed();
      await intent.submit();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Nie udało się utworzyć sesji.');
    } finally {
      setBusy(false);
      changed();
    }
  }
  const unresolved = !!client.current && client.current.state.status !== 'accepted';
  const needsMcp = providers.length>0;
  const mcpReady=!needsMcp||mcpAvailable&&!connectionsLoading&&!connectionError&&providers.every(provider=>isSecretProvider(provider)?(target==='local'||!!remoteBinding)&&secretGrants.some(g=>g.provider_id===provider&&g.id===accounts[provider]):connections.some(c=>c.provider_id===provider&&c.id===accounts[provider]&&c.status==='connected'));
  const supported = mcpReady && (target === 'local'
    ? !!catalog?.configured_harness && catalog.profiles.some(p => p.agent_id === selected && p.definition.harness === catalog.configured_harness)
    : catalog?.remote_sessions_supported === true && !!remoteBinding && catalog.profiles.some(p => p.agent_id === selected));
  return (
    <div>
      {loading ? (
        <p role="status">Wczytywanie opublikowanych profili…</p>
      ) : catalog && !catalog.profiles.length ? (
        <p>Brak opublikowanych profili w tym projekcie. Utwórz i opublikuj agenta w zakładce Agenci.</p>
      ) : null}
      {error && (
        <p role="alert" className="notice error">
          {error}
        </p>
      )}
      {catalog && catalog.profiles.length > 0 && (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            void create();
          }}
        >
          {catalog.remote_sessions_supported && (
            <label>
              Miejsce wykonania
              <select
                data-testid="session-create-target"
                value={target}
                disabled={busy || unresolved || loading}
                onChange={event => {setTarget(event.target.value);setAccounts({});}}
              >
                <option value="local">Lokalne powiązanie projektu</option>
                {bindings.filter(binding => binding.active).map(binding => (
                  <option key={binding.id} value={binding.id}>
                    Host {binding.host_id} · export {binding.export_id}
                  </option>
                ))}
              </select>
            </label>
          )}
          {bindingError && <p role="alert" className="notice error">{bindingError}</p>}
          {target !== 'local' && (
            <p>Serwer sprawdzi aktualny dostęp do hosta i zdalnego checkoutu oraz obsługę wybranych narzędzi przed uruchomieniem sesji.</p>
          )}
          <label>
            Opublikowany profil agenta
            <select
              data-testid="session-create-profile"
              required
              value={selected}
              disabled={busy || unresolved}
              onChange={(e) => {setSelected(e.target.value);setAccounts({});}}
            >
              <option value="">Wybierz profil</option>
              {catalog.profiles.map((p) => (
                <option
                  key={p.agent_id}
                  value={p.agent_id}
                  disabled={
                    !!p.definition.mcp?.length&&!mcpAvailable || target === 'local' && (!catalog.configured_harness || p.definition.harness !== catalog.configured_harness)
                  }
                >
                  {p.definition.name} · wersja {p.version}{p.definition.mcp?.length&&!mcpAvailable ? ' · MCP niedostępne' : ''}
                </option>
              ))}
            </select>
          </label>
          {target === 'local' && !catalog.configured_harness && (
            <p role="alert">
              Serwer nie ma skonfigurowanego obsługiwanego harnessu. Tworzenie sesji jest niedostępne.
            </p>
          )}
          {needsMcp&&!mcpAvailable && <p role="alert">Wybrany cel nie obsługuje wykonania MCP.</p>}
          {needsMcp&&mcpAvailable&&<fieldset disabled={busy||unresolved||connectionsLoading}>
            <legend>Prywatne połączenia MCP tej sesji</legend>
            <p>Wybierz konto lub grant poświadczenia dla każdego narzędzia. Każde wywołanie będzie wymagało Twojej jednorazowej zgody.</p>
            {connectionsLoading&&<p role="status">Wczytywanie połączeń…</p>}
            {connectionError&&<p role="alert">{connectionError}</p>}
            {providers.map(provider=><label key={provider}>{isSecretProvider(provider)?'Poświadczenie':'Konto MCP'}: {provider}
              <select required data-testid={`session-mcp-${provider}`} value={accounts[provider]??''} onChange={event=>setAccounts({...accounts,[provider]:event.target.value})}>
                <option value="">Wybierz prywatne połączenie</option>
                {isSecretProvider(provider)?secretGrants.filter(g=>g.provider_id===provider).map(g=><option key={g.id} value={g.id}>{g.credential}{g.read_only?' · tylko odczyt':''}</option>):connections.filter(c=>c.provider_id===provider&&c.status==='connected').map(c=><option key={c.id} value={c.id}>{c.display_name??c.id}</option>)}
              </select>
            </label>)}
            <button type="button" onClick={()=>setReloadConnections(value=>value+1)}>Odśwież połączenia MCP</button>
          </fieldset>}
          {target === 'local' && selected && !supported && !needsMcp && (
            <p role="alert">Wybrany profil wymaga innego harnessu niż bieżąca konfiguracja serwera.</p>
          )}
          {selected && (
            <p>
              Sesja otrzyma zapisane instrukcje, umiejętności oraz limity wybranej wersji. Serwer sprawdzi
              konfigurację modelu i trybu przed pierwszym promptem.
            </p>
          )}
          <button
            className="primary"
            data-testid="session-create-submit"
            disabled={busy || unresolved || loading || !selected || !supported}
          >
            Utwórz sesję
          </button>
        </form>
      )}
    </div>
  );
}
export function SessionCreation({
  userId,
  client,
  onChanged,
}: {
  userId: string;
  client: SessionCreationClient;
  onChanged: () => void;
}) {
  const [retrying, setRetrying] = useState(false);
  const changed = onChanged;
  const intent = client.current;
  async function retry() {
    if (!intent) return;
    setRetrying(true);
    try {
      await intent.submit();
    } catch {
      /* Intent retains authoritative error and exact request. */
    } finally {
      setRetrying(false);
      changed();
    }
  }
  return (
    <details className="session-create" data-testid="session-create">
      <summary>Utwórz sesję z opublikowanego profilu</summary>
      <AgentScope userId={userId}>
        {(w, p) => <Profiles key={`${w}:${p}`} userId={userId} workspace={w} project={p} client={client} changed={changed} />}
      </AgentScope>
      {intent && (
        <div data-testid="session-create-outcome" aria-live="polite">
          <p
            role={
              intent.state.status === 'blocked' || intent.state.status === 'uncertain' ? 'alert' : 'status'
            }
          >
            {intent.state.status === 'submitting' ? 'Oczekiwanie na przyjęcie sesji…' : intent.state.message}
          </p>
          {intent.state.status === 'uncertain' && (
            <button data-testid="session-create-retry" disabled={retrying} onClick={() => void retry()}>
              Ponów ten sam zamiar utworzenia sesji
            </button>
          )}
          {intent.state.status === 'blocked' && (
            <p>
              Nowe utworzenie jest zablokowane. Sprawdź istniejące sesje i uprawnienia; odrzuconego zamiaru
              nie ponawiamy automatycznie.
            </p>
          )}
          {intent.accepted && (
            <p>
              Identyfikator przyjętej sesji:{' '}
              <code data-testid="session-created-id">{intent.accepted.sessionId}</code>. Potwierdzenie jest
              historyczne. Wybierz sesję poniżej, aby otrzymać jej aktualny stan AHP; możliwość wysyłania
              zależy od bieżących uprawnień i działającego agenta.
            </p>
          )}
        </div>
      )}
    </details>
  );
}
