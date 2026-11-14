/** Private configuration metadata. Selection never replaces live server/SDK authorization. */
import type { AgentMcpTool } from './agents.ts';
export type ProfileRef = Readonly<{agent_id:string;version:number;digest:string}>;
export type SecretStatus = Readonly<{credential:string;revision:number;state:'installing'|'active'|'disabled'}>;
export type SecretGrant = Readonly<{id:string;credential:string;principal_kind:'human'|'service';principal_id:string;profile:ProfileRef;provider_id:string;revision:number;active:boolean;credential_revision:number;read_only:boolean}>;
export type GrantInput = Readonly<{expected_revision:number;credential_revision:number;principal_kind:'human';principal_id:string;profile:ProfileRef;provider_id:string;read_only:boolean}>;
export const SECRET_PREFIX='dunlin-secret-';
export const isSecretProvider=(value:string)=>/^dunlin-secret-[A-Za-z0-9_-]+$/.test(value)&&value.length<=64;
const object=(v:unknown):v is Record<string,unknown>=>!!v&&typeof v==='object'&&!Array.isArray(v);
const id=(v:unknown):v is string=>typeof v==='string'&&/^[A-Za-z0-9_.:-]{1,128}$/.test(v)&&v!=='.'&&v!=='..';
const pathId=(v:string)=>/^[A-Za-z0-9_-]{1,128}$/.test(v);
const rev=(v:unknown,min=1):v is number=>typeof v==='number'&&Number.isSafeInteger(v)&&v>=min;
const exact=(v:Record<string,unknown>,keys:string[])=>Object.keys(v).every(k=>keys.includes(k));
function invalid():never {throw new Error('Niepoprawna odpowiedź dotycząca poświadczeń.');}
export function profileRef(v:unknown):ProfileRef {
  if(!object(v)||!exact(v,['agent_id','version','digest'])||!id(v.agent_id)||!rev(v.version)||typeof v.digest!=='string'||!/^[a-f0-9]{64}$/.test(v.digest))return invalid();
  return {agent_id:v.agent_id,version:v.version,digest:v.digest};
}
export const sameProfile=(a:ProfileRef,b:ProfileRef)=>a.agent_id===b.agent_id&&a.version===b.version&&a.digest===b.digest;
export function decodeSecretStatus(v:unknown):SecretStatus {
  if(!object(v)||!exact(v,['credential','revision','state'])||!id(v.credential)||!rev(v.revision)||!['installing','active','disabled'].includes(String(v.state)))return invalid();
  return {credential:v.credential,revision:v.revision,state:v.state as SecretStatus['state']};
}
export function decodeSecretGrant(v:unknown):SecretGrant {
  if(!object(v)||!exact(v,['id','credential','principal_kind','principal_id','profile','provider_id','revision','active','credential_revision','read_only'])||!id(v.id)||!id(v.credential)||!id(v.principal_id)||!['human','service'].includes(String(v.principal_kind))||typeof v.provider_id!=='string'||!isSecretProvider(v.provider_id)||!rev(v.revision)||!rev(v.credential_revision)||typeof v.active!=='boolean'||typeof v.read_only!=='boolean')return invalid();
  return {id:v.id,credential:v.credential,principal_kind:v.principal_kind as SecretGrant['principal_kind'],principal_id:v.principal_id,profile:profileRef(v.profile),provider_id:v.provider_id,revision:v.revision,credential_revision:v.credential_revision,active:v.active,read_only:v.read_only};
}
function rows<T>(v:unknown,max:number,decode:(v:unknown)=>T,key:(v:T)=>string):T[] {
  if(!Array.isArray(v)||v.length>max)return invalid();const result=v.map(decode);if(new Set(result.map(key)).size!==result.length)return invalid();return result;
}
export class ScopedSecretError extends Error {
  readonly status:number;
  constructor(status:number) {super(status===401?'Sesja wygasła.':status===403||status===404?'Brak dostępu do poświadczeń w tym zakresie.':status===409?'Stan zmienił się lub wynik zapisu jest niepewny. Odśwież stan przed kolejną zmianą.':status===400?'Niepoprawna polityka lub dane poświadczenia.':'Nie udało się potwierdzić operacji poświadczenia. Odśwież stan przed kolejną zmianą.');this.status=status;}
}
function aborted(signal?:AbortSignal) {if(signal?.aborted)throw new DOMException('Aborted','AbortError');}
export class ScopedSecretsClient {
  private base:string;
  private userId:string;
  private fetcher:typeof fetch;
  constructor(workspace:string,project:string,host:string,userId:string,fetcher:typeof fetch=fetch) {
    if(![workspace,project,host].every(pathId)||!id(userId))invalid();
    this.userId=userId;this.fetcher=fetcher;
    this.base=`/v1/web/workspaces/${workspace}/projects/${project}/hosts/${host}`;
  }
  private async request(path:string,signal?:AbortSignal,body?:unknown):Promise<unknown> {
    aborted(signal);const send=this.fetcher;let response:Response;
    try {response=await send(this.base+path,{method:body===undefined?'GET':'POST',credentials:'same-origin',cache:'no-store',redirect:'error',signal,headers:{'X-Dunlin-Expected-User':this.userId,...(body===undefined?{}:{'Content-Type':'application/json'})},...(body===undefined?{}:{body:JSON.stringify(body)})});}
    catch {aborted(signal);throw new ScopedSecretError(0);}
    aborted(signal);if(!response.ok)throw new ScopedSecretError(response.status);
    if(!response.body)return invalid();const reader=response.body.getReader();let size=0;const parts:Uint8Array[]=[];
    try {while(true) {aborted(signal);const part=await reader.read();if(part.done)break;size+=part.value.length;if(size>1048576){await reader.cancel();return invalid();}parts.push(part.value);}} finally {reader.releaseLock();}
    aborted(signal);const bytes=new Uint8Array(size);let offset=0;for(const part of parts){bytes.set(part,offset);offset+=part.length;}let value:unknown;
    try {value=JSON.parse(new TextDecoder('utf-8',{fatal:true}).decode(bytes));}catch {return invalid();}aborted(signal);return value;
  }
  private credential(value:string) {if(!id(value))invalid();return `/scoped-secrets/${encodeURIComponent(value)}`;}
  async statuses(signal?:AbortSignal) {const v=await this.request('/scoped-secrets',signal);if(!object(v)||!exact(v,['credentials','execution_supported'])||typeof v.execution_supported!=='boolean')return invalid();return {credentials:rows(v.credentials,128,decodeSecretStatus,r=>r.credential),execution_supported:v.execution_supported};}
  async tool(slot:string,signal?:AbortSignal):Promise<AgentMcpTool> {
    if(!isSecretProvider(SECRET_PREFIX+slot))invalid();const v=await this.request('/scoped-secret-tool',signal);
    if(!object(v)||v.provider_prefix!==SECRET_PREFIX||!object(v.tool)||v.tool.name!=='http'||typeof v.definition_digest!=='string'||!/^[a-f0-9]{64}$/.test(v.definition_digest))return invalid();
    return {provider_id:SECRET_PREFIX+slot,name:'http',definition_digest:v.definition_digest};
  }
  async grants(credential:string,signal?:AbortSignal) {const v=await this.request(this.credential(credential)+'/grants',signal);if(!object(v)||!exact(v,['grants']))return invalid();const result=rows(v.grants,256,decodeSecretGrant,r=>r.id);if(result.some(g=>g.credential!==credential))return invalid();return result;}
  async available(profile:ProfileRef,signal?:AbortSignal) {
    const p=profileRef(profile);const query=new URLSearchParams({agent_id:p.agent_id,version:String(p.version),digest:p.digest});const v=await this.request('/secret-bindings?'+query,signal);
    if(!object(v)||!exact(v,['bindings','execution_supported'])||typeof v.execution_supported!=='boolean')return invalid();
    const result=rows(v.bindings,1024,decodeSecretGrant,r=>r.id);
    if(result.some(g=>!g.active||g.principal_kind!=='human'||g.principal_id!==this.userId||!sameProfile(g.profile,p)))return invalid();
    return {bindings:result,execution_supported:v.execution_supported};
  }
  async install(credential:string,expected:number,policy:unknown,value:string,signal?:AbortSignal) {
    if(!rev(expected,0)||expected===Number.MAX_SAFE_INTEGER||!value||new TextEncoder().encode(value).length>16384||!object(policy))invalid();
    const v=await this.request(this.credential(credential),signal,{expected_revision:expected,policy,value});
    if(!object(v)||!exact(v,['credential','revision','activation'])||v.credential!==credential||v.revision!==expected+1||v.activation!=='confirmed')return invalid();
  }
  async disable(row:SecretStatus,signal?:AbortSignal) {row=decodeSecretStatus(row);const v=decodeSecretStatus(await this.request(this.credential(row.credential)+'/disable',signal,{expected_revision:row.revision}));if(v.credential!==row.credential||v.revision!==Math.min(row.revision+1,Number.MAX_SAFE_INTEGER)||v.state!=='disabled')return invalid();}
  async setGrant(credential:string,input:GrantInput,signal?:AbortSignal) {
    profileRef(input.profile);if(!rev(input.expected_revision,0)||input.expected_revision===Number.MAX_SAFE_INTEGER||!rev(input.credential_revision)||input.principal_kind!=='human'||!id(input.principal_id)||!isSecretProvider(input.provider_id)||typeof input.read_only!=='boolean')invalid();
    const v=decodeSecretGrant(await this.request(this.credential(credential)+'/grants',signal,{expected_revision:input.expected_revision,credential_revision:input.credential_revision,principal_kind:'human',principal_id:input.principal_id,profile:input.profile,provider_id:input.provider_id,read_only:input.read_only}));
    if(v.credential!==credential||!v.active||v.revision!==input.expected_revision+1||v.credential_revision!==input.credential_revision||v.principal_kind!=='human'||v.principal_id!==input.principal_id||v.provider_id!==input.provider_id||v.read_only!==input.read_only||!sameProfile(v.profile,input.profile))return invalid();
    return v;
  }
  async revoke(row:SecretGrant,signal?:AbortSignal) {row=decodeSecretGrant(row);if(!id(row.id))invalid();const v=decodeSecretGrant(await this.request(this.credential(row.credential)+`/grants/${encodeURIComponent(row.id)}/revoke`,signal,{expected_revision:row.revision}));if(v.id!==row.id||v.credential!==row.credential||v.active||v.revision!==Math.min(row.revision+1,Number.MAX_SAFE_INTEGER)||v.principal_kind!==row.principal_kind||v.principal_id!==row.principal_id||v.provider_id!==row.provider_id||v.credential_revision!==row.credential_revision||v.read_only!==row.read_only||!sameProfile(v.profile,row.profile))return invalid();return v;}
}
