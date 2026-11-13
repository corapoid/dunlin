import { ROOT, canPredict, chat, object, reduceChat, summary } from './ahp.ts';
import type { Chat, Json, Summary } from './ahp.ts';
import { validateEndpoint } from './transport.ts';
export type ObserverState = { status: 'connecting' | 'live' | 'reconnecting' | 'offline' | 'forbidden' | 'error'; sessions: Summary[]; selected: string | null; chat: Chat | null; error: string; historyLoading: boolean; historyError: string; draft: string; pendingCount: number; commandError: string };
export function ahpEndpoint(origin: string, path = '/v1/web/ahp'): string {
  if (path !== '/v1/web/ahp') throw new Error('Nieobsługiwany endpoint AHP.');
  const url = validateEndpoint(origin); url.protocol = url.protocol === 'https:' || url.protocol === 'wss:' ? 'wss:' : 'ws:'; url.pathname = path; return url.toString();
}
const HISTORY_TAIL = 32;
type RpcFailure = { code: number; message?: string };
export class AhpObserver {
  private socket: WebSocket | null = null;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private pending = new Map<number, { done: (value: Json) => void; error?: (failure: RpcFailure) => void; timer: ReturnType<typeof setTimeout> }>();
  private id = 0;
  private clientId = crypto.randomUUID();
  private initialized = false;
  private ready = false;
  private stopped = true;
  private attempts = 0;
  private seq = 0;
  private chatSeq = 0;
  private confirmed: Chat | null = null;
  private outbox = new Map<number, { channel: string; clientSeq: number; action: Json }>();
  private drafts = new Map<string, string>();
  private nextClientSeq = 1;
  private resendNeeded = false;
  private historyEpoch = 0;
  private historyRequest: { epoch: number; channel: string; seen: boolean } | null = null;
  private listing = false;
  private listAgain = false;
  state: ObserverState = { status: 'connecting', sessions: [], selected: null, chat: null, error: '', historyLoading: false, historyError: '', draft: '', pendingCount: 0, commandError: '' };
  private origin: string;
  private change: (state: ObserverState) => void;
  private factory: (url: string) => WebSocket;
  constructor(origin: string, change: (state: ObserverState) => void, factory: (url: string) => WebSocket = (url) => new WebSocket(url)) { this.origin = origin; this.change = change; this.factory = factory; }
  private emit() {
    let predicted = this.confirmed;
    for (const item of this.outbox.values()) if (predicted && item.channel === this.state.selected && canPredict(predicted, item.action)) predicted = reduceChat(predicted, item.action);
    this.state.chat = predicted;
    this.state.draft = this.drafts.get(this.state.selected ?? '') ?? '';
    this.state.pendingCount = this.outbox.size;
    this.change({ ...this.state, sessions: [...this.state.sessions] });
  }
  canQueue(): boolean { return this.state.status === 'live' && this.confirmed?.interactivity === 'full' && this.state.sessions.some((s) => s.defaultChat === this.state.selected && s._meta?.['dunlin.queueText'] === true); }
  setDraft(text: string) { if (this.state.selected) this.drafts.set(this.state.selected, text); this.emit(); }
  private validText(text: string): boolean {
    if (!text.trim()) return false;
    if (text.length > 16_384 || new TextEncoder().encode(text).length > 32_768) { this.state.commandError = 'Wiadomość przekracza limit 32 KiB tekstu UTF-8. Skróć szkic przed wysłaniem.'; this.emit(); return false; }
    return true;
  }
  private enqueue(action: Json): boolean {
    if (!this.state.selected || this.outbox.size >= 16) return false;
    const copied = structuredClone(action);
    const freeze = (v: Json) => { for (const value of Object.values(v)) if (value && typeof value === 'object') freeze(value); Object.freeze(v); };
    freeze(copied);
    const frozen = copied;
    if (Array.isArray(frozen.order)) Object.freeze(frozen.order);
    if (frozen.message) { Object.freeze(frozen.message.origin); Object.freeze(frozen.message); }
    const item = Object.freeze({ channel: this.state.selected, clientSeq: this.nextClientSeq, action: frozen });
    if (new TextEncoder().encode(JSON.stringify({ jsonrpc: '2.0', method: 'dispatchAction', params: item })).length > 65_536) { this.state.commandError = 'Zakodowana wiadomość przekracza limit transportu 64 KiB. Skróć szkic przed wysłaniem.'; this.emit(); return false; }
    this.nextClientSeq++;
    this.outbox.set(item.clientSeq, item); this.state.commandError = ''; this.emit(); this.dispatch(item); return true;
  }
  queueText(): boolean {
    const text = this.state.draft.trim();
    if (!this.canQueue() || !this.validText(text)) return false;
    if (!this.enqueue({ type: 'chat/pendingMessageSet', kind: 'queued', id: crypto.randomUUID(), message: { text, origin: { kind: 'user' } } })) return false;
    this.drafts.delete(this.state.selected!); this.emit(); return true;
  }
  canControl(): boolean { return this.state.status === 'live' && this.confirmed?.interactivity === 'full' && this.state.sessions.some((s) => s.defaultChat === this.state.selected && s._meta?.['dunlin.controlTurns'] === true); }
  updateQueued(id: string, text: string): boolean {
    if (!this.canControl() || this.outbox.size || !this.confirmed?.queuedMessages?.some((m: Json) => m.id === id) || !this.validText(text)) return false;
    return this.enqueue({ type: 'chat/pendingMessageSet', kind: 'queued', id, message: { text, origin: { kind: 'user' } } });
  }
  removeQueued(id: string): boolean {
    if (!this.canControl() || this.outbox.size || !this.confirmed?.queuedMessages?.some((m: Json) => m.id === id)) return false;
    return this.enqueue({ type: 'chat/pendingMessageRemoved', kind: 'queued', id });
  }
  moveQueued(id: string, direction: -1 | 1): boolean {
    if (!this.canControl() || this.outbox.size) return false;
    const order: string[] = (this.confirmed?.queuedMessages ?? []).map((m: Json) => m.id);
    const index = order.indexOf(id), target = index + direction;
    if (index < 0 || target < 0 || target >= order.length) return false;
    [order[index], order[target]] = [order[target], order[index]];
    return this.enqueue({ type: 'chat/queuedMessagesReordered', order });
  }
  cancelActive(): boolean {
    if (!this.canControl() || this.outbox.size || !this.confirmed?.activeTurn) return false;
    return this.enqueue({ type: 'chat/turnCancelled', turnId: this.confirmed.activeTurn.id, duration: 0 });
  }
  canApprove(): boolean { return this.state.status === 'live' && this.confirmed?.interactivity === 'full' && this.state.sessions.some((s) => s.defaultChat === this.state.selected && s._meta?.['dunlin.approveTools'] === true); }
  confirmTool(turnId: string, toolCallId: string, selectedOptionId: string): boolean {
    if (!this.canApprove() || this.outbox.size || this.confirmed?.activeTurn?.id !== turnId) return false;
    const tool = this.confirmed.activeTurn.responseParts.find((p: Json) => p.kind === 'toolCall' && p.toolCall?.toolCallId === toolCallId)?.toolCall;
    const option = tool?.options?.find((o: Json) => o.id === selectedOptionId && ['approve', 'deny'].includes(o.kind));
    if (tool?.status !== 'pending-confirmation' || !option) return false;
    return this.enqueue({ type: 'chat/toolCallConfirmed', turnId, toolCallId, approved: option.kind === 'approve', selectedOptionId });
  }
  completeInput(requestId: string, response: 'accept' | 'decline' | 'cancel', answers?: Json): boolean {
    if (!this.canApprove() || this.outbox.size || !requestId.startsWith('dunlin:mcp-input:')) return false;
    const part = this.confirmed?.activeTurn?.responseParts.find((p: Json) => p.kind === 'inputRequest' && p.request?.id === requestId && !p.response);
    if (!part || !['accept', 'decline', 'cancel'].includes(response) || (response !== 'accept' || part.request.url !== undefined && part.request.url !== null) && answers !== undefined) return false;
    return this.enqueue({ type: 'chat/inputCompleted', requestId, response, ...(answers === undefined ? {} : { answers }) });
  }
  private historyFailure(epoch: number, channel: string, failure: RpcFailure) {
    if (this.historyEpoch !== epoch || this.state.selected !== channel) return;
    if ([-32001, -32009].includes(failure.code)) { this.fail('Dostęp do historii został odwołany.', true); return; }
    this.historyRequest = null; this.state.historyLoading = false;
    this.state.historyError = failure.code === -32602 ? 'Strona historii wygasła lub jest nieaktualna. Odśwież widok rozmowy, aby pobrać nową listę.' : 'Nie udało się wczytać historii. Odśwież widok rozmowy i spróbuj ponownie.';
    this.emit();
  }
  loadOlder(): boolean {
    const channel = this.state.selected, cursor = this.confirmed?.turnsNextCursor;
    if (!this.ready || this.state.status !== 'live' || !channel || typeof cursor !== 'string' || this.state.historyLoading || this.state.historyError) return false;
    const context = { epoch: this.historyEpoch, channel, seen: false };
    this.historyRequest = context; this.state.historyLoading = true; this.state.historyError = ''; this.emit();
    this.request('fetchTurns', { channel, cursor }, () => {
      if (this.historyEpoch !== context.epoch || this.state.selected !== channel || this.historyRequest !== context) return;
      if (!context.seen) { this.historyFailure(context.epoch, channel, { code: -32603 }); return; }
      this.historyRequest = null; this.state.historyLoading = false; this.emit();
    }, (failure) => this.historyFailure(context.epoch, channel, failure));
    return true;
  }
  refreshHistory(): boolean {
    const channel = this.state.selected;
    if (!this.ready || this.state.status !== 'live' || !channel || this.state.historyLoading) return false;
    const epoch = ++this.historyEpoch;
    this.historyRequest = null; this.state.historyLoading = true; this.state.historyError = ''; this.emit();
    this.request('subscribe', { channel, view: { turns: HISTORY_TAIL } }, (value) => {
      if (this.historyEpoch !== epoch || this.state.selected !== channel) return;
      this.snapshot(value.snapshot);
    }, (failure) => this.historyFailure(epoch, channel, failure));
    return true;
  }
  private dispatch(item: { channel: string; clientSeq: number; action: Json }) { this.socket?.send(JSON.stringify({ jsonrpc: '2.0', method: 'dispatchAction', params: item })); }
  private flushOutbox() {
    if (!this.resendNeeded || !this.ready || this.listing || !this.confirmed) return;
    this.resendNeeded = false;
    for (const item of this.outbox.values()) if (item.channel === this.state.selected) this.dispatch(item);
  }
  start() { this.stopped = false; this.connect(); }
  stop() { this.stopped = true; this.disconnect(); if (this.timer) clearTimeout(this.timer); this.timer = null; }
  private disconnect() { this.historyEpoch++; this.historyRequest = null; this.state.historyLoading = false; this.state.historyError = ''; const s = this.socket; this.socket = null; this.ready = false; for (const p of this.pending.values()) clearTimeout(p.timer); this.pending.clear(); this.listing = false; this.listAgain = false; s?.close(); }
  private fail(message: string, forbidden = false) {
    this.stop(); this.confirmed = null; this.outbox.clear(); this.drafts.clear(); this.state = { ...this.state, status: forbidden ? 'forbidden' : 'error', sessions: [], chat: null, selected: null, error: message }; this.emit();
  }
  private request(method: string, params: Json, done: (value: Json) => void, error?: (failure: RpcFailure) => void) {
    const id = ++this.id;
    this.pending.set(id, { done, error, timer: setTimeout(() => { this.pending.delete(id); if (error) error({ code: -32000 }); else this.fail('Przekroczono czas odpowiedzi AHP.'); }, 15_000) });
    this.socket?.send(JSON.stringify({ jsonrpc: '2.0', id, method, params }));
  }
  private connect() {
    if (this.stopped) return;
    this.state.status = this.initialized ? 'reconnecting' : 'connecting'; this.emit();
    let socket: WebSocket;
    try { socket = this.factory(ahpEndpoint(this.origin)); } catch { this.fail('Nie udało się połączyć z AHP.'); return; }
    this.socket = socket;
    socket.onopen = () => {
      if (this.socket !== socket) return;
      const reconnect = this.initialized;
      this.request(reconnect ? 'reconnect' : 'initialize', reconnect
        ? { channel: ROOT, clientId: this.clientId, lastSeenServerSeq: this.seq, subscriptions: [ROOT, ...(this.state.selected ? [this.state.selected] : [])] }
        : { channel: ROOT, clientId: this.clientId, protocolVersions: ['1.0.0', '0.9.0'], initialSubscriptions: [ROOT] }, (value) => {
          if (!reconnect && !['1.0.0', '0.9.0'].includes(value.protocolVersion)) throw new Error('Nieobsługiwana wersja AHP.');
          if (Number.isSafeInteger(value.serverSeq)) this.seq = Math.max(this.seq, value.serverSeq);
          if (value.type === 'replay') { for (const a of value.actions ?? []) this.action(a); if (value.missing?.includes(this.state.selected)) this.clearSelection(); }
          else { const snapshots = value.snapshots ?? []; for (const snap of snapshots) this.snapshot(snap); if (reconnect && this.state.selected && !snapshots.some((s: Json) => s.resource === this.state.selected)) this.clearSelection(); }
          this.initialized = true; this.ready = true; this.resendNeeded = reconnect; this.attempts = 0; this.state.status = 'live'; this.state.error = ''; this.emit(); this.list();
          if (this.state.selected && !this.state.chat) { const selected = this.state.selected, epoch = this.historyEpoch; this.request('subscribe', { channel: selected, view: { turns: HISTORY_TAIL } }, (result) => { if (this.state.selected === selected && this.historyEpoch === epoch) this.snapshot(result.snapshot); }); }
        });
    };
    socket.onmessage = (event) => {
      if (this.socket !== socket) return;
      try {
        if (typeof event.data !== 'string' || event.data.length > 1024 * 1024) throw new Error('Odpowiedź AHP przekracza limit widoku.');
        const m: unknown = JSON.parse(event.data);
        if (!object(m) || m.jsonrpc !== '2.0') throw new Error('Niepoprawna odpowiedź AHP.');
        if (typeof m.id === 'number') {
          const p = this.pending.get(m.id); if (!p) return; clearTimeout(p.timer); this.pending.delete(m.id);
          if (m.error?.code === -32011 && this.initialized) { this.disconnect(); this.connect(); return; }
          if (m.error?.code === -32600 && this.initialized && m.error.message === 'Initialize this client first') { if (this.outbox.size) { this.fail('Nie można potwierdzić wysłanej operacji. Sprawdź historię sesji przed ponowieniem operacji.'); return; } this.initialized = false; this.seq = 0; this.chatSeq = 0; this.confirmed = null; this.disconnect(); this.connect(); return; }
          if (m.error && p.error) { p.error(m.error); return; }
          if (m.error) { this.fail('Serwer nie udostępnił zasobu AHP.', [-32001, -32009].includes(m.error.code)); return; }
          if (!object(m.result)) throw new Error('Niepoprawna odpowiedź AHP.'); p.done(m.result);
        } else if (m.method === 'action') this.action(m.params);
        else if (m.method?.startsWith('root/session')) {
          const p = m.params;
          if (!object(p) || p.channel !== ROOT) throw new Error('Niepoprawne powiadomienie AHP.');
          if (m.method === 'root/sessionRemoved') { this.state.sessions = this.state.sessions.filter((s) => s.resource !== p.session); if (!this.state.sessions.some((s) => s.defaultChat === this.state.selected)) this.clearSelection(); }
          else if (m.method === 'root/sessionAdded') { const s = summary(p.summary); this.state.sessions = [...this.state.sessions.filter((old) => old.resource !== s.resource), s]; }
          else if (m.method === 'root/sessionSummaryChanged') this.state.sessions = this.state.sessions.map((s) => s.resource === p.session ? summary({ ...s, ...p.changes }) : s);
          if (this.listing) this.listAgain = true;
          this.emit();
        }
      } catch (error) { this.fail(error instanceof Error ? error.message : 'Błąd AHP.'); }
    };
    socket.onerror = () => { /* close is the sole reconnect trigger */ };
    socket.onclose = (event) => {
      if (this.socket !== socket) return;
      if (event.code === 1008) { this.fail('Dostęp do obserwacji został odwołany.', true); return; }
      this.disconnect(); if (this.stopped) return;
      this.attempts++;
      this.state.status = this.attempts > 8 ? 'offline' : 'reconnecting'; this.emit();
      if (this.attempts <= 8) this.timer = setTimeout(() => { this.timer = null; this.connect(); }, Math.min(1000 * 2 ** (this.attempts - 1), 30_000));
    };
  }
  private snapshot(s: Json) {
    if (!object(s) || !Number.isSafeInteger(s.fromSeq) || s.fromSeq < 0) throw new Error('Niepoprawny snapshot AHP.');
    this.seq = Math.max(this.seq, s.fromSeq);
    if (s.resource === this.state.selected) { this.confirmed = chat(s.state, s.resource); this.historyRequest = null; this.state.historyLoading = false; this.state.historyError = ''; this.chatSeq = s.fromSeq; this.emit(); this.flushOutbox(); }
  }
  private action(a: Json) {
    if (!object(a) || !Number.isSafeInteger(a.serverSeq) || a.serverSeq < 0 || !object(a.action)) throw new Error('Niepoprawna akcja AHP.');
    this.seq = Math.max(this.seq, a.serverSeq);
    // An old canonical dedupe echo can acknowledge an outbox item after a newer snapshot.
    if (a.origin?.clientId === this.clientId) {
      const item = this.outbox.get(a.origin.clientSeq);
      if (item && item.channel === a.channel) {
        this.outbox.delete(item.clientSeq);
        if (a.rejectionReason) { if (typeof item.action.message?.text === 'string') this.drafts.set(item.channel, [this.drafts.get(item.channel), item.action.message.text].filter(Boolean).join('\n')); this.state.commandError = `Operacja odrzucona: ${a.rejectionReason}`; }
      }
    }
    if (a.channel === this.state.selected && a.serverSeq > this.chatSeq && this.confirmed) { if (!a.rejectionReason) { this.confirmed = reduceChat(this.confirmed, a.action); if (a.action.type === 'chat/turnsLoaded' && this.historyRequest && this.historyRequest.channel === a.channel && this.historyRequest.epoch === this.historyEpoch) this.historyRequest.seen = true; } this.chatSeq = a.serverSeq; }
    this.emit();
  }
  private list() {
    if (!this.ready) return;
    if (this.listing) { this.listAgain = true; return; }
    this.listing = true; const items: Summary[] = []; const cursors = new Set<string>();
    const page = (cursor?: string) => this.request('listSessions', { channel: ROOT, limit: 64, ...(cursor ? { cursor } : {}) }, (result) => {
      if (!Array.isArray(result.items) || items.length + result.items.length > 4096) throw new Error('Lista sesji przekracza limit widoku.');
      items.push(...result.items.map(summary));
      if (result.nextCursor) { if (typeof result.nextCursor !== 'string' || cursors.has(result.nextCursor)) throw new Error('Niepoprawna paginacja AHP.'); cursors.add(result.nextCursor); page(result.nextCursor); }
      else { this.listing = false; this.state.sessions = items; if (this.state.selected && !items.some((s) => s.defaultChat === this.state.selected)) this.clearSelection(); this.emit(); this.flushOutbox(); if (this.listAgain) { this.listAgain = false; this.list(); } }
    }); page();
  }
  private clearSelection() { this.historyEpoch++; this.historyRequest = null; this.state.historyLoading = false; this.state.historyError = ''; for (const [seq, item] of this.outbox) if (item.channel === this.state.selected) { this.outbox.delete(seq); this.state.commandError = 'Sesja jest niedostępna. Sprawdź historię przed ponownym wysłaniem wiadomości.'; } this.drafts.delete(this.state.selected ?? ''); this.state.selected = null; this.confirmed = null; this.state.chat = null; this.chatSeq = 0; }
  select(uri: string) {
    if (!this.ready || this.state.selected === uri || !this.state.sessions.some((s) => s.defaultChat === uri)) return;
    if (this.outbox.size) { this.state.commandError = 'Poczekaj na potwierdzenie wysłanej operacji przed zmianą sesji.'; this.emit(); return; }
    const old = this.state.selected; this.clearSelection(); const epoch = this.historyEpoch; this.state.selected = uri; this.emit();
    if (old) this.socket?.send(JSON.stringify({ jsonrpc: '2.0', method: 'unsubscribe', params: { channel: old } }));
    this.request('subscribe', { channel: uri, view: { turns: HISTORY_TAIL } }, (value) => { if (this.state.selected === uri && this.historyEpoch === epoch) this.snapshot(value.snapshot); });
  }
}
