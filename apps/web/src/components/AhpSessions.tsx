import { useEffect, useState } from 'react';
import { AhpObserver } from '../lib/ahp-live';
import type { ObserverState } from '../lib/ahp-live';
import type { Json } from '../lib/ahp';
import { toolInputText, toolStatusLabel } from '../lib/ahp';
import { McpInput } from './McpInput';
import { SessionSharing } from './SessionSharing';
import { Frame, FramePanel } from './reui/frame';
const labels = { connecting: 'Łączenie…', live: 'Na żywo', reconnecting: 'Ponowne łączenie…', offline: 'Offline', forbidden: 'Brak dostępu', error: 'Błąd synchronizacji' };
function printable(value: unknown): string { return typeof value === 'string' ? value : value == null ? '' : JSON.stringify(value, null, 2); }
function Part({ part, turnId, approvals, disabled, observer }: { part: Json; turnId: string; approvals: boolean; disabled: boolean; observer: AhpObserver | null }) {
  if (part.kind === 'inputRequest') return <McpInput key={part.request?.id} part={part} allowed={approvals} disabled={disabled} observer={observer} />;
  if (part.kind === 'markdown') return <div className="ahp-text">{printable(part.content)}</div>;
  if (part.kind === 'reasoning') return <details className="ahp-detail"><summary>Rozumowanie</summary><div className="ahp-text">{printable(part.content)}</div></details>;
  if (part.kind === 'error') return <div className="notice error" role="alert">{printable(part.error?.message ?? part.error)}</div>;
  if (part.kind === 'toolCall') { const tool = part.toolCall ?? {}; return <details className="ahp-detail" open={tool.status === 'pending-confirmation'} data-testid="ahp-tool" data-tool-call-id={tool.toolCallId}><summary>{printable(tool.displayName ?? tool.toolName ?? 'Narzędzie')} · {toolStatusLabel(tool)}</summary>
    {tool.confirmationTitle && <h4>{printable(tool.confirmationTitle)}</h4>}
    {tool.intention && <p>{printable(tool.intention)}</p>}
    {tool.status === 'pending-confirmation' && <div data-testid="ahp-permission" data-tool-call-id={tool.toolCallId}><p>Zgoda dotyczy tego wywołania i pokazanych parametrów. Wynik pojawi się po wykonaniu.</p>{approvals && Array.isArray(tool.options) && tool.options.filter((o: Json) => ['approve', 'deny'].includes(o.kind) && typeof o.id === 'string' && typeof o.label === 'string').map((o: Json) => <button key={o.id} data-testid={o.kind === 'approve' ? 'ahp-tool-approve' : 'ahp-tool-deny'} data-option-id={o.id} disabled={disabled} onClick={() => observer?.confirmTool(turnId, tool.toolCallId, o.id)}>{o.label}</button>)}</div>}
    {tool.invocationMessage && <p>{printable(tool.invocationMessage)}</p>}
    {tool.pastTenseMessage && <p>{printable(tool.pastTenseMessage)}</p>}
    {tool.toolInput !== undefined && <details open={tool.status==='pending-confirmation'&&typeof tool.toolCallId==='string'&&tool.toolCallId.startsWith('dunlin:mcp:')}><summary>Parametry narzędzia</summary><pre>{toolInputText(tool.toolInput,tool.toolCallId)}</pre></details>}
    {tool.status === 'streaming' && tool.partialInput && <pre>{printable(tool.partialInput)}</pre>}
    {tool.content && <pre>{printable(tool.content)}</pre>}
    {tool.structuredContent && <details><summary>Dane wyniku</summary><pre>{printable(tool.structuredContent)}</pre></details>}
    {tool.error && <p role="alert">{printable(tool.error)}</p>}
  </details>; }
  return <details><summary>Inny fragment odpowiedzi</summary><pre>{printable(part)}</pre></details>;
}
function QueuedEntry({ message, index, count, controls, disabled, observer }: { message: Json; index: number; count: number; controls: boolean; disabled: boolean; observer: AhpObserver | null }) {
  const [editing, setEditing] = useState(false);
  const [text, setText] = useState('');
  return <div className="notice" data-testid="ahp-queued" data-message-id={message.id}>
    <p>W kolejce: {printable(message.message?.text ?? message.message)}</p>
    {controls && <>
      {editing ? <form onSubmit={(event) => { event.preventDefault(); if (observer?.updateQueued(message.id, text)) setEditing(false); }}>
        <label>Treść wiadomości<textarea data-testid="ahp-queue-editor" maxLength={16384} value={text} onChange={(event) => setText(event.target.value)} /></label>
        <button data-testid="ahp-queue-save" disabled={disabled || !text.trim()}>Zapisz zmianę</button><button type="button" onClick={() => setEditing(false)}>Zamknij edycję</button>
      </form> : <button data-testid="ahp-queue-edit" disabled={disabled} onClick={() => { setText(printable(message.message?.text)); setEditing(true); }}>Edytuj</button>}
      <button data-testid="ahp-queue-remove" disabled={disabled} onClick={() => observer?.removeQueued(message.id)}>Usuń z kolejki</button>
      <button data-testid="ahp-queue-up" disabled={disabled || index === 0} onClick={() => observer?.moveQueued(message.id, -1)}>Przesuń w górę</button>
      <button data-testid="ahp-queue-down" disabled={disabled || index === count - 1} onClick={() => observer?.moveQueued(message.id, 1)}>Przesuń w dół</button>
    </>}
  </div>;
}
export function AhpSessions({ userId }: { userId: string }) {
  const [observer, setObserver] = useState<AhpObserver | null>(null);
  const [attempt, setAttempt] = useState(0);
  const [state, setState] = useState<ObserverState>({ status: 'connecting', sessions: [], selected: null, chat: null, error: '', historyLoading: false, historyError: '', draft: '', pendingCount: 0, commandError: '' });
  useEffect(() => { const client = new AhpObserver(window.location.origin, setState); setObserver(client); client.start(); return () => client.stop(); }, [attempt]);
  const selected = state.sessions.find((s) => s.defaultChat === state.selected);
  const shared = selected?._meta?.['dunlin.shared'] === true && selected?._meta?.['dunlin.ownerId'] !== userId;
  const controls = !shared && state.chat?.interactivity === 'full' && selected?._meta?.['dunlin.controlTurns'] === true;
  const cancelClosesSession = selected?._meta?.['dunlin.cancelClosesSession'] === true;
  const approvals = !shared && state.chat?.interactivity === 'full' && selected?._meta?.['dunlin.approveTools'] === true;
  const metadata = { ...state.chat?._meta, ...selected?._meta };
  const turns = state.chat ? [...state.chat.turns, ...(state.chat.activeTurn ? [state.chat.activeTurn] : [])] : [];
  return <div data-testid="ahp-observer">
    <div className="page-heading"><div><span className="eyebrow">SESJE · AHP</span><h1>Sesje</h1><p className="muted">Obserwuj istniejące rozmowy i pracę narzędzi.</p></div><span className="preview">{shared ? 'Udostępniona · Tylko odczyt' : state.chat?.interactivity === 'full' && selected?._meta?.['dunlin.queueText'] === true ? 'Rozmowa' : 'Tylko odczyt'}</span></div>
    <p role="status" data-testid="ahp-status">{labels[state.status]}</p>
    {state.error && <p className="notice error" role="alert">{state.error}</p>}
    {state.status !== 'live' && state.chat && <p className="notice">Połączenie jest przerwane. Wyświetlany transkrypt może być nieaktualny.</p>}
    {['error', 'offline', 'forbidden'].includes(state.status) && <button onClick={() => setAttempt((n) => n + 1)}>Połącz ponownie</button>}
    <div className="ahp-layout"><Frame><FramePanel><h2>Dostępne sesje</h2>
      {state.sessions.length === 0 ? <p className="muted">{state.status === 'live' ? 'Brak dostępnych sesji. Nowe sesje pojawią się tutaj automatycznie.' : 'Oczekiwanie na sesje…'}</p> : <ul className="ahp-sessions">{state.sessions.map((s) => <li key={s.resource}><button data-testid="ahp-session" data-session-uri={s.resource} aria-pressed={state.selected === s.defaultChat} disabled={state.status !== 'live'} onClick={() => observer?.select(s.defaultChat)}><strong>{s.title || 'Sesja bez tytułu'}</strong>{s._meta?.['dunlin.shared'] === true && s._meta?.['dunlin.ownerId'] !== userId && <span>Udostępniona · Tylko odczyt</span>}<span className="muted">{s.status & 16 ? 'Oczekuje na decyzję' : s.status & 8 ? 'W toku' : s.status & 2 ? 'Wymaga uwagi' : 'Bezczynna'}</span></button></li>)}</ul>}
    </FramePanel></Frame><Frame><FramePanel><h2>{selected?.title ?? 'Rozmowa'}</h2><div id="ahp-transcript" data-testid="ahp-transcript" className="ahp-transcript" aria-busy={!!state.selected && !state.chat || state.historyLoading}>
      {!state.selected ? <p className="muted">Wybierz sesję, aby wyświetlić jej transkrypt.</p> : !state.chat ? <p role="status">Wczytywanie transkryptu…</p> : <>
        <div className="ahp-history">
          {state.chat.turnsNextCursor && <button data-testid="ahp-history-more" aria-controls="ahp-transcript" disabled={state.status !== 'live' || state.historyLoading || !!state.historyError} onClick={() => observer?.loadOlder()}>Wczytaj starsze wiadomości</button>}
          {state.historyLoading && <p role="status" aria-live="polite">Wczytywanie historii…</p>}
          {state.historyError && <><p role="alert" className="notice error">{state.historyError}</p><button data-testid="ahp-history-refresh" aria-controls="ahp-transcript" disabled={state.status !== 'live' || state.historyLoading} onClick={() => observer?.refreshHistory()}>Odśwież widok rozmowy</button><p className="muted">Odświeżenie wczyta najnowsze wiadomości. Starszą historię możesz potem pobrać ponownie.</p></>}
        </div>
        {turns.length === 0 && <p className="muted">Ta rozmowa nie zawiera jeszcze wiadomości.</p>}
        {turns.map((t, index) => <article className="ahp-turn" data-turn-id={t.id} key={t.id ?? index}>
          <h3>Użytkownik</h3><div className="ahp-text">{printable(t.message?.text ?? t.message)}</div>
          <h3>Agent {t.state === 'error' ? '· błąd' : t.state === 'cancelled' ? '· anulowano' : !t.state ? '· w toku' : ''}</h3>
          {t.responseParts.map((p: Json, i: number) => <Part key={p.id ?? i} part={p} turnId={t.id} approvals={approvals && state.chat?.activeTurn?.id === t.id} disabled={state.status !== 'live' || state.pendingCount > 0} observer={observer} />)}
        </article>)}
        {state.chat.queuedMessages?.map((m: Json, index: number) => <QueuedEntry key={m.id} message={m} index={index} count={state.chat!.queuedMessages.length} controls={controls} disabled={state.status !== 'live' || state.pendingCount > 0} observer={observer} />)}
      </>}
    </div>
    {controls && state.chat?.activeTurn && <button data-testid="ahp-cancel-turn" disabled={state.status !== 'live' || state.pendingCount > 0} onClick={() => observer?.cancelActive()}>{cancelClosesSession ? 'Anuluj i zakończ sesję' : 'Anuluj bieżącą turę'}</button>}
    {controls && cancelClosesSession && state.chat?.activeTurn && <p className="muted">Anulowanie zakończy całą zdalną sesję. Aby kontynuować, utwórz nową sesję.</p>}
    {metadata['dunlin.cancellationPending'] === true && <p role="status" className="notice" data-testid="ahp-cancel-pending">Anulowanie przyjęte. Oczekiwanie na zakończenie pracy agenta.</p>}
    {metadata['dunlin.cancellationUnconfirmed'] === true && (metadata['dunlin.remoteStopped'] === true
      ? <p role="status" className="notice" data-testid="ahp-cancel-stopped">Zatrzymanie zdalnej sesji potwierdzone. Wynik przerwanej tury pozostaje nieznany.</p>
      : <p role="alert" className="notice error" data-testid="ahp-cancel-unknown">Nie potwierdzono zakończenia pracy agenta. Wynik anulowania jest nieznany; sprawdź sesję przed ponownym uruchomieniem zadania.</p>)}
    {metadata['dunlin.permissionPending'] === true && <p role="status" className="notice" data-testid="ahp-permission-pending">Agent oczekuje na decyzję dotyczącą narzędzia lub jej przekazanie.</p>}
    {metadata['dunlin.permissionUnconfirmed'] === true && <p role="alert" className="notice error" data-testid="ahp-permission-unknown">Nie potwierdzono wykonania zatwierdzonej operacji. Wynik jest nieznany; sprawdź sesję przed ponowieniem zadania.</p>}
    {state.commandError && <p role="alert" className="notice error">{state.commandError}</p>}
    {state.pendingCount > 0 && <p role="status" data-testid="ahp-pending">Potwierdzanie operacji… ({state.pendingCount})</p>}
    {!shared && state.chat?.interactivity === 'full' && selected?._meta?.['dunlin.queueText'] === true && <form data-testid="ahp-composer" className="ahp-composer" onSubmit={(event) => { event.preventDefault(); observer?.queueText(); }}>
      <label htmlFor="ahp-prompt">Wiadomość do agenta</label>
      <textarea id="ahp-prompt" data-testid="ahp-prompt" maxLength={16384} value={state.draft} onChange={(event) => observer?.setDraft(event.target.value)} rows={3} />
      <button className="primary" data-testid="ahp-send" disabled={state.status !== 'live' || !state.draft.trim() || state.pendingCount >= 16}>Wyślij do kolejki</button>
      {state.status !== 'live' && <p className="muted">Szkic jest zachowany w tej karcie. Wysyłanie będzie dostępne po połączeniu.</p>}
    </form>}
    {selected?._meta?.['dunlin.canShare'] === true && selected?._meta?.['dunlin.ownerId'] === userId && <SessionSharing key={`${userId}:${selected.resource}`} userId={userId} sessionId={selected.resource.slice('ahp-session:/'.length)} />}
    </FramePanel></Frame></div>
  </div>;
}
