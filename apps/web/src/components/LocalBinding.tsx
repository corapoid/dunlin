import { useEffect, useRef, useState } from 'react';
import { LocalBindingController } from '../lib/local-binding';
import type { LocalBindingState } from '../lib/local-binding';
export function LocalBinding({workspace,project}:{workspace:string;project:string}) {
  const [state,setState]=useState<LocalBindingState>({status:'loading',view:null,pending:false,needsReload:false,message:''});
  const [native,setNative]=useState(''),[approval,setApproval]=useState(''),[baseline,setBaseline]=useState<number|null>(null),[error,setError]=useState('');
  const client=useRef<LocalBindingController|null>(null);
  useEffect(()=>{
    const controller=new LocalBindingController(workspace,project,s=>{
      setState(s);setError('');
      if(s.status==='ready' && !s.pending && !s.needsReload) {
        setNative(s.view?.binding?.native_project_id ?? '');setApproval(s.view?.binding?.metadata_approval_revision?.toString() ?? '');setBaseline(s.view?.binding?.revision ?? null);
      } else if(s.status!=='ready' || s.needsReload) {setNative('');setApproval('');setBaseline(null);}
    });
    client.current=controller;void controller.refresh();
    return()=>{controller.stop();if(client.current===controller)client.current=null;};
  },[workspace,project]);
  const binding=state.view?.binding,busy=state.pending || state.status==='loading',canEdit=state.status==='ready' && !state.needsReload && !!state.view?.can_manage;
  async function save() {
    try {
      const revision=approval.trim()===''?undefined:Number(approval);
      if(revision!==undefined && (!/^[1-9][0-9]*$/.test(approval) || !Number.isSafeInteger(revision)))throw new Error('Rewizja zgody musi być dodatnią bezpieczną liczbą całkowitą.');
      await client.current?.save({native_project_id:native.trim(),...(revision===undefined?{}:{metadata_approval_revision:revision})},baseline);
    } catch(cause){setError(cause instanceof Error?cause.message:'Nie udało się zapisać konfiguracji.');}
  }
  return <section data-testid="local-binding" className="team-inline-form" aria-label="Lokalny binding">
    <h3>Lokalny checkout</h3>
    <p>Binding wskazuje projekt tego hosta. Zapis konfiguracji nie uruchamia agenta ani nie nadaje grantów.</p>
    {state.message && <p role={state.status==='error'||state.status==='forbidden'?'alert':'status'}>{state.message}</p>}
    {error && <p role="alert">{error}</p>}
    {binding && <p data-testid="local-binding-saved">{binding.native_project_id} · {binding.repo_kind} · Wersja {binding.revision} · {binding.active?'aktywny':'odwołany'}{binding.metadata_approval_revision===undefined?'':` · Zgoda metadanych ${binding.metadata_approval_revision}`}</p>}
    {state.view && <p data-testid="local-binding-current">{state.view.configuration_current?'Konfiguracja bieżąca dla Twojego dostępu do odczytu.':'Konfiguracja nie została potwierdzona jako bieżąca dla Twojego dostępu.'}</p>}
    <p>Linked worktree wymaga zgody operatora przez lokalne CLI oraz Twojego osobnego grantu metadanych Read. Dostęp do wykonywania operacji wymaga odpowiednich dalszych uprawnień.</p>
    {canEdit && <>
      <label>Identyfikator projektu hosta<input data-testid="local-binding-native" value={native} maxLength={128} disabled={busy} onChange={e=>setNative(e.target.value)}/></label>
      <label>Rewizja zgody metadanych (tylko linked worktree)<input data-testid="local-binding-approval" inputMode="numeric" value={approval} maxLength={16} disabled={busy} onChange={e=>setApproval(e.target.value)}/></label>
      <button data-testid="local-binding-save" disabled={busy || !native.trim()} onClick={()=>void save()}>Zapisz lokalny binding</button>
      {binding?.active && <button data-testid="local-binding-revoke" disabled={busy} onClick={()=>void client.current?.revoke(binding.revision)}>Odwołaj lokalny binding</button>}
    </>}
    <button data-testid="local-binding-refresh" disabled={busy} onClick={()=>void client.current?.refresh()}>Odczytaj aktualną konfigurację</button>
  </section>;
}
