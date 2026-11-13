import { useEffect, useState } from 'react';
import type { FormEvent } from 'react';
import { Frame, FrameHeader, FrameTitle, FrameDescription, FramePanel } from './reui/frame';
import { decodeMembers, decodeWorkspaces, teamRequest, TeamRequestError, workspaceRoles } from '../lib/teams';
import type { Membership, Workspace } from '../lib/teams';
import { TeamProjects } from './TeamProjects';
import { AcceptTeamInvitation, ManageTeamInvitations } from './TeamInvitations';

type Props = { userId: string; canCreate: boolean; revisionKey: string; onExpired: () => void };

/** Team metadata UI. Updates follow server invalidations, never an HTTP timer. */
export function TeamSettings({ userId, canCreate, revisionKey, onExpired }: Props) {
  const [workspaces, setWorkspaces] = useState<Workspace[] | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [members, setMembers] = useState<Membership[] | null>(null);
  const [refresh, setRefresh] = useState(0);
  const [error, setError] = useState('');
  const [message, setMessage] = useState('');
  const [busy, setBusy] = useState(false);
  const workspace = workspaces?.find((item) => item.id === selected);
  const managesMembers = workspace?.membership.role === 'owner' || workspace?.membership.role === 'admin';

  function failure(cause: unknown) {
    if (cause instanceof TeamRequestError && cause.status === 401) onExpired();
    setError(cause instanceof Error ? cause.message : 'Operacja nie powiodła się.');
  }

  useEffect(() => {
    const controller = new AbortController();
    teamRequest('/v1/web/workspaces', controller.signal)
      .then((value) => {
        if (controller.signal.aborted) return;
        const next = decodeWorkspaces(value, userId);
        setWorkspaces(next);
        setSelected((current) => next.some((item) => item.id === current) ? current : null);
      }).catch((cause: unknown) => {
        if (!controller.signal.aborted) { setWorkspaces(null); setMembers(null); failure(cause); }
      });
    return () => controller.abort();
  }, [userId, revisionKey, refresh, onExpired]);

  useEffect(() => {
    setMembers(null);
    if (!workspace || !managesMembers) return;
    const controller = new AbortController();
    teamRequest(`/v1/web/workspaces/${encodeURIComponent(workspace.id)}/members`, controller.signal)
      .then((value) => { if (!controller.signal.aborted) setMembers(decodeMembers(value, workspace.id)); })
      .catch((cause: unknown) => { if (!controller.signal.aborted) failure(cause); });
    return () => controller.abort();
  }, [workspace?.id, workspace?.membership.revision, managesMembers, revisionKey, refresh, onExpired]);

  async function create(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = event.currentTarget;
    const name = String(new FormData(form).get('name') ?? '').trim();
    setBusy(true); setError(''); setMessage('');
    try {
      await teamRequest('/v1/web/workspaces', undefined, { name }, 'POST');
      form.reset(); setRefresh((value) => value + 1); setMessage('Utworzono workspace. Nie przeniesiono do niego żadnych prywatnych projektów ani sesji.');
    } catch (cause) { failure(cause); }
    finally { setBusy(false); }
  }

  async function update(event: FormEvent<HTMLFormElement>, member: Membership | null) {
    event.preventDefault();
    if (!workspace) return;
    const form = event.currentTarget;
    const values = new FormData(form);
    setBusy(true); setError(''); setMessage('');
    try {
      await teamRequest(`/v1/web/workspaces/${encodeURIComponent(workspace.id)}/members`, undefined, {
        user_id: member?.user_id ?? String(values.get('user_id') ?? '').trim(),
        role: String(values.get('role')), active: values.get('active') === 'on', expected_revision: member?.revision ?? null,
      }, 'PUT');
      if (!member) form.reset();
      setRefresh((value) => value + 1); setMessage('Zapisano członkostwo. Uprawnienia do plików hosta pozostają odrębne.');
    } catch (cause) { failure(cause); setRefresh((value) => value + 1); }
    finally { setBusy(false); }
  }

  function roleOptions() {
    return workspaceRoles.filter((role) => role !== 'owner' || workspace?.membership.role === 'owner')
      .map((role) => <option key={role} value={role}>{role}</option>);
  }

  return <section aria-label="Ustawienia zespołu" className="team-settings">
    <div className="page-heading"><div><span className="eyebrow">ZESPÓŁ</span><h1>Workspaces i członkowie</h1><p className="muted">Jawne członkostwo, wersjonowane role i prywatność sesji. Dostęp do hosta nie wynika z roli zespołowej.</p></div></div>
    {error && <p role="alert" className="notice error">{error}</p>}
    {message && <p role="status" className="notice">{message}</p>}
    <AcceptTeamInvitation key={userId} userId={userId} onExpired={onExpired} onAccepted={() => setRefresh((value) => value + 1)} />
    {canCreate && <Frame><FrameHeader><FrameTitle>Nowy workspace</FrameTitle><FrameDescription>Bootstrap dostępny wyłącznie administratorowi relay.</FrameDescription></FrameHeader><FramePanel><form onSubmit={create} className="team-inline-form"><label htmlFor="workspace-name">Nazwa</label><input id="workspace-name" name="name" required maxLength={120} /><button className="primary" disabled={busy}>Utwórz</button></form></FramePanel></Frame>}
    <Frame><FrameHeader><FrameTitle>Twoje członkostwa</FrameTitle></FrameHeader><FramePanel>
      {workspaces === null ? <p role="status">Oczekiwanie na katalog zespołów…</p> : workspaces.length === 0 ? <p className="muted">Nie należysz jeszcze do workspace. Prywatny katalog projektów nadal jest dostępny.</p> : <div className="team-workspaces">{workspaces.map((item) => <button data-testid="team-workspace" data-workspace-id={item.id} key={item.id} onClick={() => { setSelected(item.id); setError(''); setMessage(''); }} aria-pressed={selected === item.id}><strong>{item.name}</strong><span className="muted">{item.membership.role}</span></button>)}</div>}
    </FramePanel></Frame>
    {workspace && managesMembers && <ManageTeamInvitations key={`${userId}:${workspace.id}:${workspace.membership.revision}`} workspaceId={workspace.id} onExpired={onExpired} />}
    {workspace && <TeamProjects key={workspace.id} workspace={workspace} userId={userId} revisionKey={revisionKey} onExpired={onExpired} />}
    {workspace && <Frame><FrameHeader><FrameTitle>Członkowie: {workspace.name}</FrameTitle><FrameDescription>Workspace {workspace.id}</FrameDescription></FrameHeader><FramePanel>
      {!managesMembers ? <p className="muted">Twoja rola nie pozwala zarządzać listą członków.</p> : <>
        {members === null ? <p role="status">Ładowanie członków…</p> : <ul className="team-members">{members.map((member) => <li key={`${member.user_id}:${member.revision}`}><form onSubmit={(event) => update(event, member)} className="team-member-form"><div><strong>{member.user_id}</strong><span className="muted">Rewizja {member.revision}</span></div><label>Rola<select name="role" defaultValue={member.role} disabled={busy || (member.role === 'owner' && workspace.membership.role !== 'owner')}>{roleOptions()}{member.role === 'owner' && workspace.membership.role !== 'owner' && <option value="owner">owner</option>}</select></label><label className="team-checkbox"><input name="active" type="checkbox" defaultChecked={member.active} disabled={busy} />Aktywne</label><button disabled={busy || (member.role === 'owner' && workspace.membership.role !== 'owner')}>Zapisz</button></form></li>)}</ul>}
        <h2>Dodaj istniejącego użytkownika relay</h2><p className="muted">Identyfikator uzyskasz przez `dunlin-relay user list`. Możesz też utworzyć zaproszenie wymagające akceptacji odbiorcy.</p>
        <form onSubmit={(event) => update(event, null)} className="team-member-form"><label>ID użytkownika<input name="user_id" required maxLength={128} placeholder="usr-…" /></label><label>Rola<select name="role" defaultValue="member">{roleOptions()}</select></label><label className="team-checkbox"><input name="active" type="checkbox" defaultChecked />Aktywne</label><button disabled={busy}>Dodaj</button></form>
      </>}
    </FramePanel></Frame>}
  </section>;
}
