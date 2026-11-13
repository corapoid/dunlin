import { teamRequest, TeamRequestError } from './teams.ts';
export type LocalBinding = {
  workspace_id: string; project_id: string; native_project_id: string; host_id: 'local';
  repo_kind: 'directory' | 'git' | 'linked_worktree'; settings_digest: string;
  revision: number; active: boolean; metadata_approval_revision?: number;
};
export type LocalBindingView = { binding: LocalBinding | null; configuration_current: boolean; can_manage: boolean };
export type LocalBindingInput = { native_project_id: string; metadata_approval_revision?: number };
function object(v: unknown): v is Record<string, unknown> { return !!v && typeof v === 'object' && !Array.isArray(v); }
function id(v: unknown): v is string { return typeof v === 'string' && /^[A-Za-z0-9_-]{1,128}$/.test(v); }
function revision(v: unknown): v is number { return Number.isSafeInteger(v) && Number(v) >= 1; }
export function localBindingPath(workspace: string, project: string): string {
  if (!id(workspace) || !id(project)) throw new Error('Niepoprawny zakres lokalnego bindingu.');
  return `/v1/web/workspaces/${workspace}/projects/${project}/binding`;
}
export function decodeLocalBinding(v: unknown, workspace: string, project: string): LocalBinding {
  localBindingPath(workspace, project);
  if (!object(v) || Object.keys(v).some(k => !['workspace_id','project_id','native_project_id','host_id','repo_kind','settings_digest','revision','active','metadata_approval_revision'].includes(k))
    || v.workspace_id !== workspace || v.project_id !== project || !id(v.native_project_id) || v.host_id !== 'local'
    || !['directory','git','linked_worktree'].includes(String(v.repo_kind)) || typeof v.repo_kind !== 'string'
    || typeof v.settings_digest !== 'string' || !/^[a-f0-9]{64}$/.test(v.settings_digest)
    || !revision(v.revision) || typeof v.active !== 'boolean'
    || (v.metadata_approval_revision !== undefined && !revision(v.metadata_approval_revision))
    || (v.repo_kind === 'linked_worktree' && v.metadata_approval_revision === undefined)
    || (v.repo_kind !== 'linked_worktree' && v.metadata_approval_revision !== undefined))
    throw new Error('Niepoprawna lub niespójna konfiguracja lokalnego bindingu.');
  return structuredClone(v) as LocalBinding;
}
export function decodeLocalBindingView(v: unknown, workspace: string, project: string): LocalBindingView {
  if (!object(v) || Object.keys(v).some(k => !['binding','configuration_current','can_manage'].includes(k))
    || typeof v.configuration_current !== 'boolean' || typeof v.can_manage !== 'boolean')
    throw new Error('Niepoprawny stan lokalnego bindingu.');
  const binding = v.binding === null ? null : decodeLocalBinding(v.binding, workspace, project);
  if (v.configuration_current && (!binding || !binding.active)) throw new Error('Nieaktualny binding nie może być bieżącą konfiguracją.');
  return { binding, configuration_current: v.configuration_current, can_manage: v.can_manage };
}
export function decodeLocalBindingInput(v: unknown): LocalBindingInput {
  if (!object(v) || Object.keys(v).some(k => !['native_project_id','metadata_approval_revision'].includes(k)) || !id(v.native_project_id)
    || (v.metadata_approval_revision !== undefined && !revision(v.metadata_approval_revision)))
    throw new Error('Podaj identyfikator projektu hosta oraz opcjonalną dodatnią rewizję zgody metadanych.');
  return { native_project_id: v.native_project_id, ...(v.metadata_approval_revision === undefined ? {} : { metadata_approval_revision: v.metadata_approval_revision }) };
}
export async function readLocalBinding(workspace: string, project: string, signal?: AbortSignal): Promise<LocalBindingView> {
  return decodeLocalBindingView(await teamRequest(localBindingPath(workspace,project),signal),workspace,project);
}
export async function saveLocalBinding(workspace: string, project: string, input: LocalBindingInput, existing: LocalBinding | null): Promise<LocalBinding> {
  const clean=decodeLocalBindingInput(input);
  if (existing) decodeLocalBinding(existing,workspace,project);
  const saved=decodeLocalBinding(await teamRequest(localBindingPath(workspace,project),undefined,
    {...clean,expected_revision:existing?.revision ?? null},'PUT'),workspace,project);
  if (!saved.active || saved.native_project_id !== clean.native_project_id || saved.metadata_approval_revision !== clean.metadata_approval_revision)
    throw new Error('Nie potwierdzono wybranego celu lokalnego bindingu.');
  return saved;
}
export async function revokeLocalBinding(workspace: string, project: string, existing: LocalBinding): Promise<LocalBinding> {
  decodeLocalBinding(existing,workspace,project);
  const saved=decodeLocalBinding(await teamRequest(`${localBindingPath(workspace,project)}/revoke`,undefined,{expected_revision:existing.revision},'POST'),workspace,project);
  if (saved.active || saved.native_project_id !== existing.native_project_id) throw new Error('Nie potwierdzono odwołania lokalnego bindingu.');
  return saved;
}
export type LocalBindingState = { status: 'loading' | 'ready' | 'error' | 'forbidden'; view: LocalBindingView | null; pending: boolean; needsReload: boolean; message: string };
/** Each edit uses one explicit CAS baseline. Invalidations fence both read and
 * mutation receipts; unknown results require a fresh explicit read, never retry. */
export class LocalBindingController {
  #generation=0; #stopped=false; #pending=false; #read: AbortController | null=null;
  #workspace: string; #project: string; #changed: (s: LocalBindingState)=>void;
  #state: LocalBindingState={status:'loading',view:null,pending:false,needsReload:false,message:''};
  constructor(workspace:string,project:string,changed:(s:LocalBindingState)=>void) {localBindingPath(workspace,project);this.#workspace=workspace;this.#project=project;this.#changed=changed;}
  get state() {return this.#state;}
  #publish(s:LocalBindingState) {if(!this.#stopped){this.#state={...s,pending:this.#pending};this.#changed(this.#state);}}
  stop() {this.#stopped=true;this.#generation++;this.#read?.abort();}
  async refresh() {
    if(this.#stopped)return;
    const generation=++this.#generation;this.#read?.abort();const abort=new AbortController();this.#read=abort;
    this.#publish({status:'loading',view:null,pending:this.#pending,needsReload:true,message:''});
    try {
      const view=await readLocalBinding(this.#workspace,this.#project,abort.signal);
      if(!this.#stopped && generation===this.#generation) this.#publish({status:'ready',view,pending:this.#pending,needsReload:false,message:''});
    } catch(error) {
      if(!this.#stopped && generation===this.#generation && !abort.signal.aborted) this.#failed(error,false);
    }
  }
  #failed(error:unknown,mutation:boolean) {
    const denied=error instanceof TeamRequestError && [401,403,404].includes(error.status);
    this.#publish({status:denied?'forbidden':'error',view:null,pending:this.#pending,needsReload:true,message:denied?'Brak aktualnego dostępu do lokalnego bindingu.':mutation?'Nie potwierdzono wyniku zmiany. Odczytaj zapisany stan przed nową decyzją; nie ponawiamy zmiany automatycznie.':'Nie udało się odczytać lokalnego bindingu. Odśwież dane.'});
  }
  async #mutate(expected:number|null,operation:(existing:LocalBinding|null)=>Promise<LocalBinding>):Promise<LocalBinding|null> {
    const view=this.#state.view;
    if(this.#stopped || this.#pending || this.#state.status!=='ready' || this.#state.needsReload || !view?.can_manage
      || (view.binding?.revision ?? null)!==expected) throw new Error('Wczytaj aktualną konfigurację przed zmianą.');
    const generation=++this.#generation;this.#read?.abort();this.#pending=true;
    this.#publish({...this.#state,pending:true,message:''});
    try {
      const binding=await operation(view.binding);
      if(this.#stopped || generation!==this.#generation)return null;
      this.#publish({status:'ready',view:{binding,configuration_current:false,can_manage:false},pending:true,needsReload:true,message:'Zapisano konfigurację. Odczytaj jej aktualny stan przed kolejną zmianą.'});
      return binding;
    } catch(error) {
      if(!this.#stopped && generation===this.#generation)this.#failed(error,true);
      return null;
    } finally {this.#pending=false;if(!this.#stopped)this.#publish({...this.#state,pending:false});}
  }
  save(input:LocalBindingInput,expected:number|null) {const clean=decodeLocalBindingInput(input);return this.#mutate(expected,existing=>saveLocalBinding(this.#workspace,this.#project,clean,existing));}
  revoke(expected:number) {return this.#mutate(expected,existing=>{if(!existing)throw new Error('Brak zapisanego bindingu.');return revokeLocalBinding(this.#workspace,this.#project,existing);});}
}
