import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { FormEvent } from 'react';
import { Frame, FrameHeader, FrameTitle, FrameDescription, FramePanel } from './reui/frame';
import { decodeWorkspaces, teamRequest, TeamRequestError } from '../lib/teams';
import type { Workspace } from '../lib/teams';
import { McpOAuthClient, McpRequestError, rememberMcpReturn } from '../lib/mcp-oauth';
import type { McpCatalog, McpConnection, McpProvider } from '../lib/mcp-oauth';
const labels:Record<McpConnection['status'],string>={connected:'Połączono',authorization_incomplete:'Logowanie nieukończone',reauthorization_required:'Wymaga ponownego logowania',disabled:'Odłączono w Dunlin'};
type Data={workspace:string;providers:McpProvider[];connections:McpConnection[]};
export function McpConnections({userId,onExpired,initialWorkspace,revisionKey}:{userId:string;onExpired:()=>void;initialWorkspace?:string;revisionKey:string}) {
  const [workspaces,setWorkspaces]=useState<Workspace[]>([]),[workspace,setWorkspace]=useState(''),[data,setData]=useState<Data|null>(null);
  const [provider,setProvider]=useState(''),[displayName,setDisplayName]=useState(''),[loading,setLoading]=useState(true),[busy,setBusy]=useState(false);
  const [error,setError]=useState(''),[notice,setNotice]=useState(''),[reload,setReload]=useState(0),[uncertain,setUncertain]=useState(false);
  const [workspacesLoading,setWorkspacesLoading]=useState(true);
  const [catalog,setCatalog]=useState<McpCatalog|null>(null);
  const alive=useRef(true);
  const mutation=useRef<AbortController|null>(null),busyRef=useRef(false);
  useLayoutEffect(()=>{alive.current=true;return()=>{alive.current=false;mutation.current?.abort();};},[]);
  useEffect(()=>{
    const controller=new AbortController();setWorkspacesLoading(true);setData(null);
    teamRequest('/v1/web/workspaces',controller.signal).then(value=>{
      if(controller.signal.aborted||!alive.current)return;const rows=decodeWorkspaces(value,userId);setWorkspaces(rows);
      setWorkspace(rows.some(w=>w.id===initialWorkspace)?initialWorkspace!:rows.length===1?rows[0].id:'');
    }).catch(cause=>{if(!controller.signal.aborted&&alive.current){setError('Nie udało się wczytać zespołów.');if(cause instanceof TeamRequestError&&cause.status===401)onExpired();}})
      .finally(()=>{if(!controller.signal.aborted&&alive.current)setWorkspacesLoading(false);});
    return()=>controller.abort();
  },[userId,initialWorkspace,onExpired]);
  useEffect(()=>{
    mutation.current?.abort();busyRef.current=false;setBusy(false);setData(null);setCatalog(null);setProvider('');setDisplayName('');setError('');
    if(!workspace){setLoading(false);return;}const controller=new AbortController();setLoading(true);
    Promise.resolve().then(()=>{const client=new McpOAuthClient(workspace,userId,window.location.origin);return Promise.all([client.providers(controller.signal),client.connections(controller.signal)]);}).then(([providers,connections])=>{
      if(controller.signal.aborted||!alive.current)return;setData({workspace,providers,connections});setUncertain(false);
      if(providers.length){setProvider(providers[0].id);setDisplayName(providers[0].name);}
    }).catch(cause=>{if(!controller.signal.aborted&&alive.current){setError(cause instanceof McpRequestError?cause.message:'Nie udało się odświeżyć połączeń.');if(cause instanceof McpRequestError&&cause.status===403)setData(null);if(cause instanceof McpRequestError&&cause.status===401)onExpired();}})
      .finally(()=>{if(!controller.signal.aborted&&alive.current)setLoading(false);});
    return()=>{controller.abort();mutation.current?.abort();};
  },[workspace,userId,reload,onExpired]);
  useEffect(()=>{
    if(!workspace)return;const controller=new AbortController();
    teamRequest('/v1/web/workspaces',controller.signal).then(value=>{
      if(controller.signal.aborted||!alive.current)return;
      const rows=decodeWorkspaces(value,userId);setWorkspaces(rows);
      if(!rows.some(row=>row.id===workspace)){mutation.current?.abort();setData(null);setWorkspace('');setNotice('Dostęp do zespołu zmienił się. Wybierz inny zespół.');}
    }).catch(cause=>{if(!controller.signal.aborted&&alive.current&&cause instanceof TeamRequestError&&cause.status===401)onExpired();});
    return()=>controller.abort();
  },[revisionKey,workspace,userId,onExpired]);
  async function run(action:(client:McpOAuthClient,signal:AbortSignal)=>Promise<void>) {
    if(busyRef.current||!workspace||!data||data.workspace!==workspace||uncertain)return;
    const controller=new AbortController();mutation.current=controller;busyRef.current=true;setBusy(true);setError('');setNotice('');
    try{await action(new McpOAuthClient(workspace,userId,window.location.origin),controller.signal);}
    catch(cause){if(!controller.signal.aborted&&alive.current){setUncertain(true);setError(cause instanceof McpRequestError?cause.message:'Nie udało się potwierdzić wyniku. Odśwież listę przed kolejną próbą.');if(cause instanceof McpRequestError&&cause.status===403)setData(null);if(cause instanceof McpRequestError&&cause.status===401)onExpired();}}
    finally{if(mutation.current===controller){busyRef.current=false;if(!controller.signal.aborted&&alive.current)setBusy(false);}}
  }
  async function connect(event:FormEvent) {
    event.preventDefault();const name=displayName.trim();if(!name)return;await authorize(provider,undefined,name);
  }
  async function authorize(providerId:string,connection?:McpConnection,name?:string) {
    await run(async(client,signal)=>{
      const started=await client.authorize(providerId,name,signal,connection);if(signal.aborted||!alive.current)return;
      rememberMcpReturn(userId,workspace);window.location.assign(started.authorization_url);
    });
  }
  async function disconnect(connection:McpConnection) {
    await run(async(client,signal)=>{
      await client.disconnect(connection,signal);if(signal.aborted||!alive.current)return;
      setNotice('Odłączono w Dunlin. Zgodę możesz również cofnąć w ustawieniach konta u dostawcy.');setReload(n=>n+1);
    });
  }
  async function revoke(connection:McpConnection) {
    await run(async(client,signal)=>{
      const result=await client.revoke(connection,signal);if(signal.aborted||!alive.current)return;
      setNotice(result.state==='confirmed_known_tokens'?'Dostawca potwierdził odwołanie zapisanych tokenów. Połączenie jest odłączone.':'Połączenie jest odłączone. Odwołanie tokenów u dostawcy pozostaje niepotwierdzone.');setReload(n=>n+1);
    });
  }
  async function reconcileRevocation(connection:McpConnection) {
    await run(async(client,signal)=>{
      const result=await client.reconcileRevocation(connection,signal);if(signal.aborted||!alive.current)return;
      setNotice(result.state==='confirmed_known_tokens'?'Zakończono zapis operacji. Dostawca potwierdził odwołanie zapisanych tokenów.':'Zakończono zapis przerwanej operacji. Wynik u dostawcy pozostaje niepotwierdzony. Możesz ponownie się połączyć lub jawnie ponowić odwołanie.');setReload(n=>n+1);
    });
  }
  async function cleanup(connection:McpConnection) {
    await run(async(client,signal)=>{
      const result=await client.cleanup(connection,signal);if(signal.aborted||!alive.current)return;
      setNotice((result.reclaimed??0)>0?`Zwolnione miejsca na kolejne zapisy: ${result.reclaimed}.`:result.status==='sweep_complete'?'Serwer potwierdził usunięcia w tym przebiegu sprzątania.':'Część usunięć nie została potwierdzona. Możesz ponowić sprzątanie.');setReload(n=>n+1);
    });
  }
  async function inspect(connection:McpConnection) {
    setCatalog(null);
    await run(async(client,signal)=>{const result=await client.inspect(connection,signal);if(!signal.aborted&&alive.current)setCatalog(result);});
  }
  const current=data?.workspace===workspace?data:null;
  const blocked=loading||workspacesLoading||busy||uncertain||!current;
  return <div className="mcp-connections">
    <div className="page-heading"><div><span className="eyebrow">TWOJE KONTA</span><h1>Połączenia MCP</h1><p className="muted">Połącz swoje konto z dostawcą udostępnionym przez administratora. Połączenia są przypisane do Twojego konta i zespołu.</p></div></div>
    <div className="agent-scope"><label>Zespół<select data-testid="mcp-workspace" value={workspace} disabled={busy||workspacesLoading} onChange={e=>{mutation.current?.abort();setNotice('');setUncertain(false);setWorkspace(e.target.value);}}><option value="">Wybierz zespół</option>{workspaces.map(w=><option key={w.id} value={w.id}>{w.name}</option>)}</select></label>
      <button onClick={()=>{mutation.current?.abort();setReload(n=>n+1);}} disabled={!workspace||loading||busy}>Odśwież listę</button></div>
    {(loading||workspacesLoading)&&<p role="status">Wczytywanie połączeń…</p>}
    {error&&<p role="alert" className="notice error">{error}</p>}
    {notice&&<p role="status" className="notice">{notice}</p>}
    {uncertain&&<p className="muted">Kolejne operacje są wstrzymane do odświeżenia listy. Żądanie nie zostanie automatycznie powtórzone.</p>}
    {!loading&&!workspacesLoading&&!workspaces.length&&!error&&<p>Brak dostępnych zespołów.</p>}
    {current&&<>
      <Frame><FrameHeader><FrameTitle>Nowe połączenie</FrameTitle><FrameDescription>Logowanie odbędzie się na stronie wybranego dostawcy.</FrameDescription></FrameHeader><FramePanel>
        {current.providers.length?<form className="pairing-form" onSubmit={connect}>
          <label htmlFor="mcp-provider">Dostawca</label><select id="mcp-provider" data-testid="mcp-provider" value={provider} disabled={blocked} onChange={e=>{setProvider(e.target.value);setDisplayName(current.providers.find(p=>p.id===e.target.value)?.name??'');}}>{current.providers.map(p=><option key={p.id} value={p.id}>{p.name}</option>)}</select>
          <label htmlFor="mcp-name">Nazwa połączenia</label><input id="mcp-name" data-testid="mcp-display-name" value={displayName} onChange={e=>setDisplayName(e.target.value)} maxLength={120} required disabled={blocked} placeholder="Np. konto służbowe"/>
          <button className="primary" data-testid="mcp-connect" disabled={blocked||!provider||!displayName.trim()}>{busy?'Oczekiwanie…':'Przejdź do logowania'}</button>
        </form>:<p>Administrator nie udostępnił jeszcze żadnego dostawcy MCP.</p>}
      </FramePanel></Frame>
      <div className="mcp-connection-list">{current.connections.length===0?<p>Nie masz jeszcze połączeń w tym zespole.</p>:current.connections.map(connection=>{
        const configured=current.providers.find(p=>p.id===connection.provider_id);
        return <Frame key={connection.id} data-testid="mcp-connection"><FrameHeader><FrameTitle>{connection.display_name??configured?.name??'Połączenie MCP'}</FrameTitle><FrameDescription>{labels[connection.status]}</FrameDescription></FrameHeader><FramePanel>
          {connection.display_name&&configured&&<p className="muted">{configured.name}</p>}
          <div className="host-actions">
          {connection.status==='connected'&&configured&&<button disabled={blocked} onClick={()=>inspect(connection)}>Sprawdź narzędzia</button>}
          {connection.status==='disabled'?configured?<button disabled={blocked||!!connection.revocation&&connection.revocation.state==='unfinished'} onClick={()=>authorize(configured.id,connection)}>Połącz ponownie</button>:<p className="muted">Dostawca nie jest obecnie dostępny. Skontaktuj się z administratorem.</p>
            :<button disabled={blocked} onClick={()=>disconnect(connection)}>{connection.status==='authorization_incomplete'?'Anuluj logowanie':'Odłącz w Dunlin'}</button>}
          {configured&&(!connection.revocation||connection.revocation.state==='unconfirmed'||connection.revocation.state==='confirmed_known_tokens'&&connection.revocation.revision<connection.revision)&&<button disabled={blocked} onClick={()=>revoke(connection)}>{connection.revocation?.state==='unconfirmed'&&connection.revocation.revision===connection.revision?'Ponów odwołanie tokenów':'Odwołaj tokeny u dostawcy'}</button>}
          {connection.revocation?.state==='unfinished'&&connection.revocation.reconciliation_supported===true&&connection.revocation.revision===connection.revision&&<button disabled={blocked} onClick={()=>reconcileRevocation(connection)}>Rozstrzygnij przerwane odwołanie</button>}
          {(connection.storage?.cleanup_available||connection.revocation&&connection.revocation.confirmed>0)&&connection.status!=='authorization_incomplete'&&<button disabled={blocked} onClick={()=>cleanup(connection)}>{connection.cleanup?'Ponów sprzątanie odwołanych tokenów':'Usuń odwołane tokeny z serwera'}</button>}
          </div>
          {connection.storage&&<p role="status" data-testid="mcp-storage">Wolne miejsca na zapisy poświadczeń: {connection.storage.available_slots} z 256. Zachowana historia sprzątania: {connection.storage.retained_history}.{connection.storage.uncertain_history>0&&' Niepewne wpisy nie wygasają automatycznie.'}{connection.storage.uncertain_history===256&&connection.storage.available_slots===0&&' Limit niepewnej historii został osiągnięty. Rozstrzygnij stare zapisy z administratorem.'}</p>}
          {connection.cleanup&&<p role="status" data-testid="mcp-cleanup">{connection.cleanup.status==='sweep_complete'?'Serwer potwierdził usunięcia odwołanych tokenów w ostatnim przebiegu. Możesz ponowić sprzątanie, aby usunąć spóźnione zapisy.':connection.cleanup.status==='unfinished'?'Brak potwierdzenia zakończenia sprzątania. Odśwież listę i jawnie ponów przebieg.':'Nie wszystkie usunięcia zostały potwierdzone. Możesz jawnie ponowić sprzątanie.'}</p>}
          {configured&&(!connection.revocation||connection.revocation.revision<connection.revision)&&<p className="muted">Odwołanie tokenów odłączy połączenie w Dunlin i wyśle żądania do dostawcy. Pełną zgodę na dostęp sprawdzisz w ustawieniach swojego konta u dostawcy.</p>}
          {connection.revocation&&connection.revocation.revision===connection.revision&&<p role="status" data-testid="mcp-revocation">{connection.revocation.state==='confirmed_known_tokens'?'Dostawca potwierdził odwołanie zapisanych tokenów. Nie oznacza to potwierdzenia cofnięcia całej zgody na koncie.':connection.revocation.state==='unfinished'?'Brak potwierdzonego zakończenia odwołania. Operacja mogła zostać przerwana. Połączenie pozostaje odłączone; żądania nie są ponawiane automatycznie. Ponowne logowanie jest wstrzymane. Rozstrzygnięcie wymaga potwierdzenia zakończenia poprzedniego wykonawcy.':'Nie udało się potwierdzić odwołania wszystkich tokenów. Połączenie pozostaje odłączone. Sprawdź zgodę w ustawieniach dostawcy. Możesz jawnie ponowić odwołanie lub rozpocząć nowe logowanie.'}</p>}
          {connection.revocation?.state==='unfinished'&&connection.revocation.reconciliation_supported!==true&&<p className="muted">Ta operacja pochodzi ze starszej wersji bez potwierdzenia zakończenia wykonawcy. Administrator musi przeprowadzić odzyskiwanie po zatrzymaniu starej instancji.</p>}
          {connection.status==='connected'&&catalog?.connection_id===connection.id&&catalog.revision===connection.revision&&<div role="status" data-testid="mcp-catalog"><p>MCP {catalog.protocol_version} · Narzędzia: {catalog.tools.length}</p><p className="muted">Katalog dostawcy opisuje dostępne narzędzia. Uprawnienia do ich wykonania są sprawdzane osobno.</p>{catalog.tools.length>0&&<details><summary>Nazwy narzędzi</summary><ul>{catalog.tools.map(name=><li key={name}><code>{name}</code>{catalog.tool_pins?.find(pin=>pin.name===name)&&<details><summary>Przypięcie do profilu agenta</summary><pre style={{whiteSpace:'pre-wrap',overflowWrap:'anywhere'}}>{JSON.stringify(catalog.tool_pins.find(pin=>pin.name===name),null,2)}</pre></details>}</li>)}</ul></details>}</div>}
          <details><summary>Szczegóły połączenia</summary><p className="muted">Identyfikator do kontaktu z administratorem: <code>{connection.id}</code></p></details>
        </FramePanel></Frame>;
      })}</div>
    </>}
  </div>;
}
