import { useEffect, useRef, useState } from 'react';
import { AgentScope } from './AgentScope';
import { readHosts, type Host } from '../lib/hosts';
import { readProfiles, type AgentVersion, type AgentMcpTool } from '../lib/agents';
import { decodeMembers, teamRequest, type Membership } from '../lib/teams';
import { ScopedSecretsClient, SECRET_PREFIX, isSecretProvider, sameProfile, type SecretStatus, type SecretGrant } from '../lib/scoped-secrets';

export function ScopedToolPicker({workspace,project,userId,onPin,disabled=false}:{workspace:string;project:string;userId:string;onPin:(pin:AgentMcpTool)=>void;disabled?:boolean}) {
  const [open,setOpen]=useState(false),[hosts,setHosts]=useState<Host[]>([]),[host,setHost]=useState(''),[slot,setSlot]=useState(''),[error,setError]=useState(''),[busy,setBusy]=useState(false);
  const alive=useRef(true);useEffect(()=>{alive.current=true;return()=>{alive.current=false;};},[]);
  useEffect(()=>{if(!open)return;const c=new AbortController();readHosts(workspace,c.signal).then(rows=>{if(!c.signal.aborted)setHosts(rows.filter(h=>h.status==='approved'));}).catch(()=>{if(!c.signal.aborted)setError('Katalog poświadczeń wymaga dostępu do zarządzania hostami.');});return()=>c.abort();},[open,workspace]);
  async function add() {setBusy(true);setError('');try {const pin=await new ScopedSecretsClient(workspace,project,host,userId).tool(slot);if(alive.current){onPin(pin);setSlot('');}}catch(e){if(alive.current)setError(e instanceof Error?e.message:'Nie udało się odczytać narzędzia.');}finally{if(alive.current)setBusy(false);}}
  return <details onToggle={e=>setOpen(e.currentTarget.open)}><summary>Dodaj dostęp HTTP z poświadczeniem</summary><p>Po opublikowaniu profilu przypisz poświadczenie w zakładce Poświadczenia. Właściciel sesji wybierze grant dla hosta wykonania.</p>{error&&<p role="alert">{error}</p>}<label>Host katalogu<select data-testid="scoped-tool-host" value={host} onChange={e=>setHost(e.target.value)} disabled={disabled||busy}><option value="">Wybierz cel</option><option value="local">Lokalny kontroler · powiązanie projektu</option>{hosts.map(h=><option value={h.id} key={h.id}>{h.definition.name}</option>)}</select></label><label>Nazwa dostępu HTTP<input data-testid="scoped-tool-slot" maxLength={50} pattern="[A-Za-z0-9_\-]+" value={slot} onChange={e=>setSlot(e.target.value)} disabled={disabled||busy} placeholder="np. github"/></label><button type="button" data-testid="scoped-tool-add" disabled={disabled||busy||!host||!isSecretProvider(SECRET_PREFIX+slot)} onClick={()=>void add()}>Dodaj narzędzie HTTP do profilu</button></details>;
}
function HostSecrets({workspace,project,host,userId}:{workspace:string;project:string;host:string;userId:string}) {
  const client=useRef(new ScopedSecretsClient(workspace,project,host,userId)).current;
  const [statuses,setStatuses]=useState<SecretStatus[]>([]),[profiles,setProfiles]=useState<AgentVersion[]>([]),[members,setMembers]=useState<Membership[]>([]),[grants,setGrants]=useState<SecretGrant[]>([]);
  const [selected,setSelected]=useState(''),[name,setName]=useState(''),[domain,setDomain]=useState(''),[prefix,setPrefix]=useState('/'),[advanced,setAdvanced]=useState('');
  const [profileId,setProfileId]=useState(''),[principal,setPrincipal]=useState(userId),[slot,setSlot]=useState(''),[readOnly,setReadOnly]=useState(true);
  const [reload,setReload]=useState(0),[loading,setLoading]=useState(true),[busy,setBusy]=useState(false),[needsReload,setNeedsReload]=useState(false),[error,setError]=useState(''),[message,setMessage]=useState('');
  const secret=useRef<HTMLInputElement>(null),alive=useRef(true),mutation=useRef<AbortController|null>(null);
  useEffect(()=>{alive.current=true;return()=>{alive.current=false;mutation.current?.abort();if(secret.current)secret.current.value='';};},[]);
  useEffect(()=>{
    const c=new AbortController();setLoading(true);setError('');setStatuses([]);setGrants([]);setProfiles([]);setMembers([]);
    Promise.all([client.statuses(c.signal),readProfiles(workspace,project,c.signal),teamRequest(`/v1/web/workspaces/${workspace}/members`,c.signal).then(v=>decodeMembers(v,workspace))]).then(async([list,catalog,people])=>{
      const saved=selected?await client.grants(selected,c.signal):[];
      if(!c.signal.aborted){setStatuses(list.credentials);setProfiles(catalog.profiles);setMembers(people.filter(m=>m.active));setGrants(saved);setNeedsReload(false);}
    }).catch(e=>{if(!c.signal.aborted){setNeedsReload(true);setError(e instanceof Error?e.message:'Nie udało się odczytać poświadczeń.');}}).finally(()=>{if(!c.signal.aborted)setLoading(false);});
    return()=>c.abort();
  },[client,workspace,project,selected,reload]);
  const credential=statuses.find(s=>s.credential===selected),profile=profiles.find(p=>p.agent_id===profileId),providers=profile?.definition.mcp?.filter(p=>isSecretProvider(p.provider_id))??[];
  const ref=profile?{agent_id:profile.agent_id,version:profile.version,digest:profile.digest}:undefined;
  const previous=ref?grants.find(g=>g.principal_kind==='human'&&g.principal_id===principal&&g.provider_id===slot&&sameProfile(g.profile,ref)):undefined;
  async function mutate(action:(signal:AbortSignal)=>Promise<unknown>,success:string) {
    const controller=new AbortController();mutation.current=controller;setBusy(true);setNeedsReload(true);setError('');setMessage('');
    try {await action(controller.signal);if(alive.current){setMessage(success);setReload(n=>n+1);}}
    catch(e){if(alive.current){setError(e instanceof Error?e.message:'Nie udało się potwierdzić zmiany.');setStatuses([]);setGrants([]);}}
    finally{if(alive.current)setBusy(false);if(mutation.current===controller)mutation.current=null;}
  }
  function save() {
    const value=secret.current?.value??'';if(secret.current)secret.current.value='';
    const id=credential?.credential??name;
    if(!credential&&statuses.some(s=>s.credential===id)){setError('Wybierz istniejące poświadczenie z listy, aby je zmienić.');return;}
    let policy:unknown;
    try {policy=advanced.trim()?JSON.parse(advanced):{version:1,managed:true,injection:{type:'header',name:'Authorization',prefix:'Bearer '},allowed_hosts:[domain.trim()],path_prefix:prefix,allowed_methods:['GET','HEAD'],allowed_headers:[],auto_approve:false,expires_at:null,project_ids:[project],recipes:[],operations:[]};}
    catch {setError('Polityka musi być poprawnym dokumentem JSON. Wprowadź wartość ponownie po poprawieniu polityki.');return;}
    void mutate(signal=>client.install(id,credential?.revision??0,policy,value,signal),'Poświadczenie zapisano. Wartość została usunięta z formularza. Przypisz dostęp do opublikowanego profilu.');
  }
  const disabled=busy||loading||needsReload;
  return <section className="scoped-secrets" data-testid="scoped-secrets-editor"><h2>Poświadczenia projektu na wybranym hoście</h2><p>Wartość nie jest odczytywana po zapisie. Zmiana polityki wymaga podania nowej wartości. Każde użycie HTTP wymaga osobnej zgody właściciela sesji.</p>
    {error&&<p role="alert">{error}</p>}{message&&<p role="status">{message}</p>}{loading&&<p role="status">Wczytywanie poświadczeń…</p>}
    <button type="button" data-testid="scoped-reload" disabled={busy||loading} onClick={()=>setReload(n=>n+1)}>Odśwież stan poświadczeń</button>
    {needsReload&&!loading&&<p role="alert">Odśwież stan przed kolejną zmianą. Nie ponawiamy zapisu automatycznie.</p>}
    <label>Poświadczenie<select data-testid="scoped-credential" value={selected} disabled={disabled} onChange={e=>{setSelected(e.target.value);setMessage('');setAdvanced('');setDomain('');if(secret.current)secret.current.value='';}}><option value="">Nowe poświadczenie</option>{statuses.map(s=><option key={s.credential} value={s.credential}>{s.credential} · {s.state==='active'?'aktywne':s.state==='disabled'?'wyłączone':'zapis niepotwierdzony'} · wersja {s.revision}</option>)}</select></label>
    <form onSubmit={e=>{e.preventDefault();save();}}><fieldset disabled={disabled||credential?.state==='installing'}><legend>{credential?'Zastąp wartość i politykę':'Zapisz poświadczenie'}</legend>
      {!selected&&<label>Nazwa poświadczenia<input data-testid="scoped-name" required maxLength={128} pattern="[A-Za-z0-9_:\-][A-Za-z0-9_.:\-]*" value={name} onChange={e=>setName(e.target.value)}/></label>}
      <label>Wartość poświadczenia<input data-testid="scoped-value" ref={secret} type="password" required maxLength={16384} autoComplete="off" spellCheck={false}/></label>
      <label>Dozwolony host HTTPS<input data-testid="scoped-domain" required={!advanced.trim()} value={domain} onChange={e=>setDomain(e.target.value)} placeholder="api.example.com" maxLength={253}/></label>
      <label>Dozwolony prefiks ścieżki<input data-testid="scoped-path" required={!advanced.trim()} value={prefix} onChange={e=>setPrefix(e.target.value)} maxLength={2048}/></label>
      <p>Domyślnie: nagłówek Authorization: Bearer, wyłącznie GET i HEAD.</p>
      <details><summary>Zaawansowana polityka HTTP</summary><p>Pełna polityka zastępuje powyższe ustawienia. Umożliwia inne metody, sposób uwierzytelnienia i nazwane operacje. Zakres projektu musi pozostać bieżący.</p><textarea data-testid="scoped-policy" value={advanced} onChange={e=>setAdvanced(e.target.value)} maxLength={32768} rows={6}/></details>
      <button data-testid="scoped-save" className="primary">{credential?'Zastąp poświadczenie':'Zapisz poświadczenie'}</button></fieldset></form>
    {credential&&<div><p>Wersja {credential.revision}. {credential.state==='installing'?'Zapis trwa lub wynik jest niepewny. Wyłączenie odetnie możliwość aktywacji tego zapisu.':''}</p><button data-testid="scoped-disable" disabled={disabled||credential.state==='disabled'} onClick={()=>void mutate(signal=>client.disable(credential,signal),'Poświadczenie wyłączono. Powiązane sesje utracą dostęp przy sprawdzeniu uprawnień.')}>Wyłącz poświadczenie</button></div>}
    {credential&&<><h3>Dostęp profili i użytkowników</h3><ul data-testid="scoped-grants">{grants.map(g=><li key={g.id}>{g.principal_id} · {g.profile.agent_id} v{g.profile.version} · {g.provider_id} · {g.active&&g.credential_revision===credential.revision?'przyznany':'odwołany lub wymaga ponownego przypisania'}{g.read_only?' · tylko odczyt':''}<button type="button" data-testid={`scoped-revoke-${g.id}`} disabled={disabled||!g.active} onClick={()=>void mutate(signal=>client.revoke(g,signal),'Grant odwołano.')}>Odwołaj grant</button></li>)}</ul>
      <form onSubmit={e=>{e.preventDefault();if(!ref||!credential)return;void mutate(signal=>client.setGrant(credential.credential,{expected_revision:previous?.revision??0,credential_revision:credential.revision,principal_kind:'human',principal_id:principal,profile:ref,provider_id:slot,read_only:readOnly},signal),'Grant zapisano dla wskazanej wersji profilu i poświadczenia.');}}><fieldset disabled={disabled||credential.state!=='active'}><legend>Przypisz dostęp</legend><p>Użytkownik musi mieć prawo wyświetlania projektu i tworzenia sesji. Zmiana wersji profilu lub poświadczenia wymaga ponownego przypisania.</p>
        <label>Opublikowany profil<select data-testid="scoped-profile" required value={profileId} onChange={e=>{setProfileId(e.target.value);setSlot('');setReadOnly(true);}}><option value="">Wybierz profil</option>{profiles.filter(p=>p.definition.mcp?.some(t=>isSecretProvider(t.provider_id))).map(p=><option key={p.agent_id} value={p.agent_id}>{p.definition.name} · wersja {p.version}</option>)}</select></label>
        <label>Użytkownik<select data-testid="scoped-principal" required value={principal} onChange={e=>{setPrincipal(e.target.value);setReadOnly(true);}}>{members.map(m=><option key={m.user_id} value={m.user_id}>{m.user_id}{m.user_id===userId?' (Ty)':''}</option>)}</select></label>
        <label>Dostęp HTTP profilu<select data-testid="scoped-slot" required value={slot} onChange={e=>{setSlot(e.target.value);setReadOnly(true);}}><option value="">Wybierz dostęp</option>{providers.map(p=><option key={p.provider_id} value={p.provider_id}>{p.provider_id}</option>)}</select></label>
        <label className="team-checkbox"><input data-testid="scoped-readonly" type="checkbox" checked={readOnly} onChange={e=>setReadOnly(e.target.checked)}/>Tylko odczyt HTTP</label>
        <button data-testid="scoped-grant-save">{previous?'Zastąp grant':'Przyznaj dostęp'}</button></fieldset></form></>}
  </section>;
}
function ProjectSecrets({workspace,project,userId}:{workspace:string;project:string;userId:string}) {
  const [hosts,setHosts]=useState<Host[]>([]),[host,setHost]=useState(''),[error,setError]=useState('');
  useEffect(()=>{const c=new AbortController();readHosts(workspace,c.signal).then(rows=>{if(!c.signal.aborted)setHosts(rows.filter(h=>h.status==='approved'));}).catch(e=>{if(!c.signal.aborted)setError(e instanceof Error?e.message:'Nie udało się odczytać hostów.');});return()=>c.abort();},[workspace]);
  return <>{error&&<p role="alert">{error}</p>}<label>Host poświadczeń<select data-testid="scoped-host" value={host} onChange={e=>setHost(e.target.value)}><option value="">Wybierz cel</option><option value="local">Lokalny kontroler · powiązanie projektu</option>{hosts.map(h=><option key={h.id} value={h.id}>{h.definition.name}</option>)}</select></label>{host&&<HostSecrets key={`${workspace}:${project}:${host}:${userId}`} workspace={workspace} project={project} host={host} userId={userId}/>}</>;
}
export function ScopedSecrets({userId}:{userId:string}) {return <section><h1>Poświadczenia</h1><AgentScope userId={userId}>{(workspace,project)=><ProjectSecrets key={`${workspace}:${project}:${userId}`} workspace={workspace} project={project} userId={userId}/>}</AgentScope></section>;}
