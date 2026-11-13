import { decodeMcpTool, type AgentMcpTool } from './agents.ts';
export type McpProvider = { id: string; name: string };
export type McpRevocation = { state:'unfinished'|'unconfirmed'|'confirmed_known_tokens'; revision:number; confirmed:number; unconfirmed:number; reconciliation_supported?:boolean };
export type McpCleanup = { revision:number; connection_revision:number; status:'unfinished'|'sweep_complete'|'unconfirmed'; delete_acknowledged:number; unconfirmed:number; reclaimed?:number; pruned?:number };
export type McpStorage = {pending_references:number;retained_history:number;uncertain_history:number;available_slots:number;cleanup_available:boolean};
export type McpConnection = { id: string; revision: number; status: 'connected'|'authorization_incomplete'|'reauthorization_required'|'disabled'; provider_id?: string; display_name?: string; revocation?: McpRevocation; cleanup?: McpCleanup; storage?:McpStorage };
export type Authorization = { authorization_url: string; connection_id: string; revision: number };
export type McpCatalog = { connection_id:string; revision:number; protocol_version:string; tools:string[]; tool_pins?:AgentMcpTool[] };
const statuses = ['connected','authorization_incomplete','reauthorization_required','disabled'];
function object(value: unknown): value is Record<string,unknown> { return !!value && typeof value==='object' && !Array.isArray(value); }
function id(value: unknown): value is string { return typeof value==='string' && value.length>0 && value.length<=128 && /^[A-Za-z0-9_.:-]+$/.test(value) && value!=='.' && value!=='..'; }
function providerId(value: unknown): value is string { return typeof value==='string' && /^[A-Za-z0-9_-]{1,64}$/.test(value); }
function revision(value: unknown): value is number { return Number.isSafeInteger(value) && Number(value)>0; }
function name(value: unknown): value is string { return typeof value==='string' && value.trim()===value && value.length>0 && [...value].length<=120 && !/[\u0000-\u001f\u007f-\u009f]/u.test(value); }
function invalid(): never { throw new Error('Niepoprawna odpowiedź dotycząca połączenia MCP.'); }
export function decodeProviders(value: unknown): McpProvider[] {
  if (!object(value)||!Array.isArray(value.providers)||value.providers.length>16) return invalid();
  const ids=new Set<string>();
  return value.providers.map((p:unknown)=>{if(!object(p)||!providerId(p.id)||!name(p.name)||ids.has(p.id))return invalid();ids.add(p.id);return {id:p.id,name:p.name};});
}
function counts(value:Record<string,unknown>) {return Number.isSafeInteger(value.confirmed)&&Number(value.confirmed)>=0&&Number.isSafeInteger(value.unconfirmed)&&Number(value.unconfirmed)>=0&&Number(value.confirmed)+Number(value.unconfirmed)<=256;}
function decodeRevocation(value:unknown):McpRevocation {
  if(!object(value)||!['unfinished','unconfirmed','confirmed_known_tokens'].includes(String(value.state))||!revision(value.revision)||!counts(value)
    ||value.state==='confirmed_known_tokens'&&(Number(value.confirmed)===0||value.unconfirmed!==0)
    ||value.reconciliation_supported!==undefined&&(typeof value.reconciliation_supported!=='boolean'||value.state!=='unfinished'))return invalid();
  return {state:value.state as McpRevocation['state'],revision:value.revision,confirmed:value.confirmed as number,unconfirmed:value.unconfirmed as number,...(value.reconciliation_supported===undefined?{}:{reconciliation_supported:value.reconciliation_supported as boolean})};
}
function decodeStorage(value:unknown):McpStorage {
  if(!object(value))return invalid();
  for(const key of ['pending_references','retained_history','uncertain_history','available_slots'])if(!Number.isSafeInteger(value[key])||Number(value[key])<0)return invalid();
  if(Number(value.pending_references)>256||Number(value.available_slots)!==256-Number(value.pending_references)||Number(value.uncertain_history)>256||Number(value.uncertain_history)>Number(value.retained_history)||Number(value.retained_history)-Number(value.uncertain_history)>768||typeof value.cleanup_available!=='boolean')return invalid();
  return {pending_references:value.pending_references as number,retained_history:value.retained_history as number,uncertain_history:value.uncertain_history as number,available_slots:value.available_slots as number,cleanup_available:value.cleanup_available};
}
function decodeCleanup(value:unknown):McpCleanup {
  if(!object(value)||!revision(value.revision)||!revision(value.connection_revision)||!['unfinished','sweep_complete','unconfirmed'].includes(String(value.status))
    ||!Number.isSafeInteger(value.delete_acknowledged)||Number(value.delete_acknowledged)<0||!Number.isSafeInteger(value.unconfirmed)||Number(value.unconfirmed)<0
    ||Number(value.delete_acknowledged)+Number(value.unconfirmed)<1||Number(value.delete_acknowledged)+Number(value.unconfirmed)>256
    ||value.status==='sweep_complete'&&value.unconfirmed!==0||value.status==='unconfirmed'&&value.unconfirmed===0
    ||value.reclaimed!==undefined&&(!Number.isSafeInteger(value.reclaimed)||Number(value.reclaimed)<0||Number(value.reclaimed)>Number(value.delete_acknowledged))
    ||value.pruned!==undefined&&(!Number.isSafeInteger(value.pruned)||Number(value.pruned)<0||Number(value.pruned)>1280))return invalid();
  return {revision:value.revision,connection_revision:value.connection_revision,status:value.status as McpCleanup['status'],delete_acknowledged:value.delete_acknowledged as number,unconfirmed:value.unconfirmed as number,...(value.reclaimed===undefined?{}:{reclaimed:value.reclaimed as number}),...(value.pruned===undefined?{}:{pruned:value.pruned as number})};
}
export function decodeConnection(value:unknown): McpConnection {
  if (!object(value)||!id(value.id)||!revision(value.revision)||!statuses.includes(String(value.status))
    ||value.provider_id!==undefined&&!providerId(value.provider_id)||value.display_name!==undefined&&!name(value.display_name)) return invalid();
  const revocation=value.revocation===undefined?undefined:decodeRevocation(value.revocation);
  if(revocation&&(revocation.revision>value.revision||revocation.revision===value.revision&&revocation.state!=='confirmed_known_tokens'&&value.status!=='disabled'))return invalid();
  const cleanup=value.cleanup===undefined?undefined:decodeCleanup(value.cleanup);
  if(cleanup&&cleanup.connection_revision>value.revision)return invalid();
  const storage=value.storage===undefined?undefined:decodeStorage(value.storage);
  return {id:value.id,revision:value.revision,...(storage===undefined?{}:{storage}),...(cleanup===undefined?{}:{cleanup}),status:value.status as McpConnection['status'],...(revocation===undefined?{}:{revocation}),
    ...(value.provider_id===undefined?{}:{provider_id:value.provider_id as string}),...(value.display_name===undefined?{}:{display_name:value.display_name as string})};
}
export function decodeConnections(value:unknown): McpConnection[] {
  if (!object(value)||!Array.isArray(value.connections)||value.connections.length>64) return invalid();
  const ids=new Set<string>();return value.connections.map((item:unknown)=>{const row=decodeConnection(item);if(ids.has(row.id))return invalid();ids.add(row.id);return row;});
}
export function decodeCatalog(value:unknown,connection:McpConnection):McpCatalog {
  if(!object(value)||value.connection_id!==connection.id||value.revision!==connection.revision
    ||!['2026-07-28','2025-03-26','2025-06-18','2025-11-25'].includes(String(value.protocol_version))
    ||!Array.isArray(value.tools)||value.tools.length>512
    ||value.tools.some(tool=>typeof tool!=='string'||!tool.length||tool.length>256||/[\u0000-\u001f\u007f-\u009f]/u.test(tool))
    ||new Set(value.tools).size!==value.tools.length)return invalid();
  let pins:AgentMcpTool[]|undefined;
  if(value.tool_pins!==undefined){
    if(!Array.isArray(value.tool_pins)||value.tool_pins.length>value.tools.length)return invalid();
    pins=value.tool_pins.map(decodeMcpTool);
    if(new Set(pins.map(pin=>pin.name)).size!==pins.length||pins.some(pin=>pin.provider_id!==connection.provider_id||!(value.tools as string[]).includes(pin.name)))return invalid();
  }
  return {connection_id:connection.id,revision:connection.revision,protocol_version:value.protocol_version as string,tools:value.tools as string[],...(pins===undefined?{}:{tool_pins:pins})};
}
function secure(url:URL): boolean {
  const host=url.hostname.replace(/^\[|\]$/g,'');
  return !url.username&&!url.password&&!url.hash&&(url.protocol==='https:'||url.protocol==='http:'&&(host==='localhost'||host==='::1'||/^127(?:\.\d{1,3}){3}$/.test(host)));
}
export function decodeAuthorization(value:unknown,provider:string,origin:string,existing?:McpConnection):Authorization {
  if(!object(value)||!id(value.connection_id)||!revision(value.revision)||typeof value.authorization_url!=='string'||value.authorization_url.length>65536||!providerId(provider))return invalid();
  if(existing&&(value.connection_id!==existing.id||value.revision<existing.revision||value.revision>existing.revision+1))return invalid();
  let url:URL;try{url=new URL(value.authorization_url);}catch{return invalid();}
  if(!secure(url))return invalid();
  const one=(key:string)=>{const values=url.searchParams.getAll(key);return values.length===1?values[0]:undefined;};
  const redirect=one('redirect_uri');let callback:URL;try{callback=new URL(redirect??'');}catch{return invalid();}
  if(!secure(callback)||callback.origin!==new URL(origin).origin||callback.pathname!==`/v1/web/mcp/oauth/${provider}/callback`||callback.search
    ||one('response_type')!=='code'||!one('client_id')||one('code_challenge_method')!=='S256'
    ||!/^[-_A-Za-z0-9]{43}$/.test(one('state')??'')||!/^[-_A-Za-z0-9]{43}$/.test(one('code_challenge')??'')
    ||['client_secret','code_verifier','access_token','refresh_token'].some(key=>url.searchParams.has(key)))return invalid();
  return {authorization_url:url.href,connection_id:value.connection_id,revision:value.revision};
}
export class McpRequestError extends Error {
  readonly status:number;
  constructor(status:number) {super(status===401?'Sesja wygasła.':status===403?'Dostęp do zespołu został odwołany.':status===409?'Stan połączenia zmienił się. Odśwież listę.':status===429?'Trwa inna autoryzacja lub osiągnięto limit. Odśwież listę przed kolejną próbą.':'Nie udało się wykonać operacji połączenia.');this.status=status;}
}
function aborted(signal?:AbortSignal) { if(signal?.aborted)throw new DOMException('Aborted','AbortError'); }
export class McpOAuthClient {
  private base:string;
  private origin:string;
  private userId:string;
  private fetcher:typeof fetch;
  constructor(workspace:string,userId:string,origin:string,fetcher:typeof fetch=fetch) {if(!id(workspace)||!id(userId))invalid();this.userId=userId;this.base=`/v1/web/workspaces/${encodeURIComponent(workspace)}`;this.origin=origin;this.fetcher=fetcher;}
  private async request(path:string,signal?:AbortSignal,body?:unknown):Promise<unknown> {
    aborted(signal);
    // Native Window.fetch must not receive this client as its receiver.
    const send=this.fetcher;
    const response=await send(this.base+path,{method:body===undefined?'GET':'POST',credentials:'same-origin',cache:'no-store',redirect:'error',signal,
      headers:{'X-Dunlin-Expected-User':this.userId,...(body===undefined?{}:{'Content-Type':'application/json'})},...(body===undefined?{}:{body:JSON.stringify(body)})});
    aborted(signal);if(!response.ok)throw new McpRequestError(response.status);
    if(!response.body)return invalid();
    const reader=response.body.getReader();const parts:Uint8Array[]=[];let length=0;
    try {while(true){aborted(signal);const part=await reader.read();if(part.done)break;length+=part.value.length;if(length>262144){await reader.cancel();return invalid();}parts.push(part.value);}}
    finally {reader.releaseLock();}
    aborted(signal);const bytes=new Uint8Array(length);let offset=0;for(const part of parts){bytes.set(part,offset);offset+=part.length;}
    let value:unknown;try{value=JSON.parse(new TextDecoder('utf-8',{fatal:true}).decode(bytes));}catch{return invalid();}aborted(signal);return value;
  }
  async providers(signal?:AbortSignal) {return decodeProviders(await this.request('/mcp-providers',signal));}
  async connections(signal?:AbortSignal) {return decodeConnections(await this.request('/mcp-connections',signal));}
  async authorize(provider:string,displayName:string|undefined,signal?:AbortSignal,existing?:McpConnection) {
    if(!providerId(provider)||displayName!==undefined&&!name(displayName))return invalid();
    if(existing&&(existing.status!=='disabled'||existing.revocation&&existing.revocation.state==='unfinished'))return invalid();
    const body=existing?{connection_id:existing.id,expected_revision:existing.revision}:displayName===undefined?{}:{display_name:displayName};
    return decodeAuthorization(await this.request(`/mcp-providers/${provider}/authorize`,signal,body),provider,this.origin,existing);
  }
  async disconnect(connection:McpConnection,signal?:AbortSignal) {
    if(!id(connection.id)||!revision(connection.revision))return invalid();
    const value=await this.request(`/mcp-connections/${encodeURIComponent(connection.id)}/disconnect`,signal,{expected_revision:connection.revision});
    if(!object(value)||value.provider_revocation!=='not_confirmed')return invalid();
    const row=decodeConnection(value.connection);
    if(row.id!==connection.id||row.status!=='disabled'||row.revision!==connection.revision+1)return invalid();
    return row;
  }
  async revoke(connection:McpConnection,signal?:AbortSignal) {
    if(!id(connection.id)||!revision(connection.revision)||connection.revocation&&
      (connection.revocation.state==='unfinished'||connection.revocation.state==='confirmed_known_tokens'&&connection.revocation.revision===connection.revision))return invalid();
    const value=await this.request(`/mcp-connections/${encodeURIComponent(connection.id)}/revoke`,signal,{expected_revision:connection.revision});
    if(!object(value)||value.connection_id!==connection.id||!['confirmed_known_tokens','unconfirmed'].includes(String(value.provider_revocation)))return invalid();
    const receipt=decodeRevocation({...value,state:value.provider_revocation});
    if(receipt.revision!==connection.revision+1)return invalid();
    return receipt;
  }
  async reconcileRevocation(connection:McpConnection,signal?:AbortSignal) {
    if(!id(connection.id)||!revision(connection.revision)||connection.status!=='disabled'||connection.revocation?.state!=='unfinished'||connection.revocation.reconciliation_supported!==true||connection.revocation.revision!==connection.revision)return invalid();
    const value=await this.request(`/mcp-connections/${encodeURIComponent(connection.id)}/reconcile-revocation`,signal,{expected_revision:connection.revision});
    if(!object(value)||value.connection_id!==connection.id||!['confirmed_known_tokens','unconfirmed'].includes(String(value.provider_revocation)))return invalid();
    const receipt=decodeRevocation({...value,state:value.provider_revocation});
    if(receipt.revision!==connection.revision||receipt.confirmed!==connection.revocation.confirmed||receipt.unconfirmed!==connection.revocation.unconfirmed)return invalid();
    return receipt;
  }
  async cleanup(connection:McpConnection,signal?:AbortSignal) {
    if(!id(connection.id)||!revision(connection.revision)||!(connection.storage?.cleanup_available||connection.revocation&&connection.revocation.confirmed>0))return invalid();
    const value=await this.request(`/mcp-connections/${encodeURIComponent(connection.id)}/cleanup`,signal,{expected_revision:connection.revision,expected_cleanup_revision:connection.cleanup?.revision??0});
    if(!object(value)||value.connection_id!==connection.id)return invalid();
    const receipt=decodeCleanup(value);
    if(receipt.connection_revision!==connection.revision||receipt.revision!==(connection.cleanup?.revision??0)+1||receipt.status==='unfinished')return invalid();
    return receipt;
  }
  async inspect(connection:McpConnection,signal?:AbortSignal) {
    if(!id(connection.id)||!revision(connection.revision)||connection.status!=='connected')return invalid();
    return decodeCatalog(await this.request(`/mcp-connections/${encodeURIComponent(connection.id)}/inspect`,signal,{expected_revision:connection.revision}),connection);
  }
}
const RETURN_KEY='dunlin.mcp.return';
export function rememberMcpReturn(userId:string,workspace:string) {try{if(id(userId)&&id(workspace))sessionStorage.setItem(RETURN_KEY,JSON.stringify({userId,workspace}));}catch{/* UI preference is optional; no credential is stored. */}}
export function consumeMcpReturn(userId:string):string|undefined {
  try{const raw=sessionStorage.getItem(RETURN_KEY);sessionStorage.removeItem(RETURN_KEY);if(!raw||raw.length>512)return;const value:unknown=JSON.parse(raw);if(object(value)&&value.userId===userId&&id(value.workspace))return value.workspace;}catch{/* Invalid or unavailable UI preference is discarded. */}
}
