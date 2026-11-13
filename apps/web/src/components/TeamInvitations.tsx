import { useEffect, useRef, useState } from 'react';
import type { FormEvent } from 'react';
import { Frame, FrameHeader, FrameTitle, FrameDescription, FramePanel } from './reui/frame';
import { decodeMembership, teamRequest, TeamRequestError, workspaceRoles } from '../lib/teams';

type Props = { userId: string; onExpired: () => void; onAccepted: () => void };

/** Tokens live only in the mounted form; never put them in a link or storage. */
export function AcceptTeamInvitation({ userId, onExpired, onAccepted }: Props) {
  const pending = useRef<AbortController | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [message, setMessage] = useState('');
  useEffect(() => () => pending.current?.abort(), [userId]);

  async function accept(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = event.currentTarget;
    const token = String(new FormData(form).get('token') ?? '').trim();
    form.reset();
    pending.current?.abort();
    const controller = new AbortController();
    pending.current = controller;
    setBusy(true); setError(''); setMessage('');
    try {
      const member = decodeMembership(await teamRequest('/v1/web/invitations/accept', controller.signal, { token }, 'POST'));
      if (controller.signal.aborted) return;
      if (member.user_id !== userId || !member.active) throw new Error('Niespójny odbiorca zaproszenia.');
      setMessage('Dołączono do workspace.');
      onAccepted();
    } catch (cause) {
      if (controller.signal.aborted) return;
      if (cause instanceof TeamRequestError && cause.status === 401) onExpired();
      setError(cause instanceof TeamRequestError && cause.status === 403
        ? 'Zaproszenie wygasło, zostało odwołane lub jest przeznaczone dla innej osoby.'
        : cause instanceof Error ? cause.message : 'Nie udało się przyjąć zaproszenia.');
    } finally { if (!controller.signal.aborted) setBusy(false); }
  }

  return <Frame><FrameHeader><FrameTitle>Przyjmij zaproszenie</FrameTitle><FrameDescription>Użyj kodu otrzymanego od administratora zespołu. Zaproszenie musi wskazywać Twoje konto.</FrameDescription></FrameHeader><FramePanel>
    {error && <p role="alert" className="notice error">{error}</p>}
    {message && <p role="status" className="notice">{message}</p>}
    <form onSubmit={accept} className="team-inline-form" autoComplete="off">
      <label htmlFor="invitation-token">Kod zaproszenia</label><input id="invitation-token" name="token" type="password" required maxLength={256} autoComplete="off" spellCheck={false} />
      <button className="primary" disabled={busy}>Przyjmij zaproszenie</button>
    </form>
  </FramePanel></Frame>;
}

/** Mount only while the current user may manage this workspace. */
export function ManageTeamInvitations({ workspaceId, onExpired }: { workspaceId: string; onExpired: () => void }) {
  const pending = useRef<AbortController | null>(null);
  const [busy, setBusy] = useState(false);
  const [token, setToken] = useState('');
  const [error, setError] = useState('');
  const [message, setMessage] = useState('');
  useEffect(() => () => pending.current?.abort(), [workspaceId]);

  async function submit(event: FormEvent<HTMLFormElement>, revoke: boolean) {
    event.preventDefault();
    const form = event.currentTarget;
    const data = new FormData(form);
    const user = String(data.get('user_id') ?? '').trim();
    const controller = new AbortController();
    pending.current?.abort(); pending.current = controller;
    setBusy(true); setToken(''); setError(''); setMessage('');
    try {
      const path = `/v1/web/workspaces/${encodeURIComponent(workspaceId)}/invitations`;
      const result = await teamRequest(revoke ? `${path}/${encodeURIComponent(user)}` : path, controller.signal,
        revoke ? undefined : { user_id: user, role: String(data.get('role')) }, revoke ? 'DELETE' : 'POST');
      if (controller.signal.aborted) return;
      if (revoke) setMessage('Odwołano zaproszenia wskazanego użytkownika.');
      else {
        if (typeof result !== 'object' || result === null || !('token' in result)
          || typeof result.token !== 'string' || !/^kst_[A-Za-z0-9_-]{43}$/.test(result.token)) throw new Error('Niepoprawna odpowiedź zaproszenia.');
        setToken(result.token);
      }
      form.reset();
    } catch (cause) {
      if (controller.signal.aborted) return;
      if (cause instanceof TeamRequestError && cause.status === 401) onExpired();
      setError(cause instanceof Error ? cause.message : 'Nie udało się zmienić zaproszeń.');
    } finally { if (!controller.signal.aborted) setBusy(false); }
  }

  return <Frame><FrameHeader><FrameTitle>Zaproszenia do zespołu</FrameTitle><FrameDescription>Kod jest ważny przez 24 godziny i działa tylko na wskazanym koncie relay. Przekaż go odbiorcy prywatnym kanałem.</FrameDescription></FrameHeader><FramePanel>
    {error && <p role="alert" className="notice error">{error}</p>}
    {message && <p role="status" className="notice">{message}</p>}
    <form onSubmit={(event) => submit(event, false)} className="team-member-form">
      <label>Odbiorca zaproszenia<input name="user_id" required maxLength={128} placeholder="usr-…" /></label>
      <label>Rola zaproszenia<select name="role" defaultValue="member">{workspaceRoles.filter((role) => role !== 'owner').map((role) => <option key={role}>{role}</option>)}</select></label>
      <button disabled={busy}>Utwórz zaproszenie</button>
    </form>
    {token && <div className="notice"><label>Kod dla odbiorcy<input value={token} readOnly autoComplete="off" spellCheck={false} /></label><p>Skopiuj kod teraz. Po opuszczeniu tego panelu nie będzie ponownie widoczny.</p><button onClick={() => setToken('')}>Ukryj kod</button></div>}
    <form onSubmit={(event) => submit(event, true)} className="team-inline-form">
      <label htmlFor="revoke-recipient">Odbiorca do odwołania</label><input id="revoke-recipient" name="user_id" required maxLength={128} />
      <button disabled={busy}>Odwołaj zaproszenia</button>
    </form>
  </FramePanel></Frame>;
}
