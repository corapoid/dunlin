/** Small observer adapter for the pinned AHP 1.0 wire contract (vendor/ahp-*).
 * It deliberately exposes no dispatch, creation or approval methods. */
export type Json = { [key: string]: any }; // Wire objects are validated at the boundary below.
export const ROOT = 'ahp-root://';
export type Summary = Json & { resource: string; title: string; defaultChat: string; status: number };
export type Chat = Json & { resource: string; turns: Json[]; activeTurn?: Json; interactivity: 'read-only' | 'full' };
export function object(value: unknown): value is Json { return !!value && typeof value === 'object' && !Array.isArray(value); }
export function summary(value: unknown): Summary {
  if (!object(value) || typeof value.resource !== 'string' || !value.resource.startsWith('ahp-session:')
    || typeof value.title !== 'string' || typeof value.status !== 'number' || !Number.isSafeInteger(value.status) || typeof value.defaultChat !== 'string' || !value.defaultChat.startsWith('ahp-chat:')) throw new Error('Niepoprawna sesja AHP.');
  return value as Summary;
}
function validTurn(value: unknown): boolean { return object(value) && typeof value.id === 'string' && Array.isArray(value.responseParts) && value.responseParts.length <= 8192 && value.responseParts.every((p: unknown) => object(p) && typeof p.kind === 'string'); }
function cursor(value: unknown): boolean { return value === undefined || typeof value === 'string' && value.length > 0 && value.length <= 512 && !/[\u0000-\u001f\u007f]/.test(value); }
export function chat(value: unknown, resource: string): Chat {
  if (!object(value) || value.resource !== resource || !Array.isArray(value.turns) || value.turns.length > 256
    || (value.interactivity !== undefined && !['read-only', 'full'].includes(value.interactivity)) || value.turns.some((t: unknown) => !validTurn(t))
    || (value.activeTurn && !validTurn(value.activeTurn))
    || !cursor(value.turnsNextCursor)
    || (value.queuedMessages && (!Array.isArray(value.queuedMessages) || value.queuedMessages.some((m: unknown) => !object(m))))) throw new Error('Niepoprawny transkrypt AHP.');
  return { ...value, interactivity: value.interactivity ?? 'full' } as Chat;
}
/** Content references are displayed as text only; this observer does not fetch URIs. */
export function toolInputText(value: unknown, toolCallId?: unknown): string {
  // Presentation only: the complete immutable invocation remains in AHP.
  if (typeof toolCallId === 'string' && toolCallId.startsWith('dunlin:mcp:')) {
    let invocation = value;
    if (typeof invocation === 'string' && invocation.length <= 64 * 1024) {
      try { invocation = JSON.parse(invocation); } catch { return 'Parametry narzędzia niedostępne.'; }
    }
    return object(invocation) && object(invocation.arguments)
      ? JSON.stringify(invocation.arguments, null, 2)
      : 'Parametry narzędzia niedostępne.';
  }
  if (typeof value === 'string') return value;
  if (object(value) && typeof value.uri === 'string') return `Odwołanie do parametrów: ${value.uri}${typeof value.contentType === 'string' ? ` (${value.contentType})` : ''}${typeof value.sizeHint === 'number' ? ` · ${value.sizeHint} B` : ''}`;
  return '';
}
export function toolStatusLabel(tool: Json): string { return tool.status === 'pending-confirmation' ? 'oczekuje na decyzję' : tool.status === 'cancelled' ? tool.reason === 'denied' ? 'odrzucone' : tool.reason === 'skipped' ? 'pominięte' : 'anulowane' : tool.status === 'completed' ? tool.success === false ? 'błąd' : 'zakończone' : 'w toku'; }
function toolBase(tool: Json): Json {
  const base: Json = { toolCallId: tool.toolCallId, toolName: tool.toolName, displayName: tool.displayName };
  for (const key of ['intention', 'contributor', '_meta']) if (tool[key] !== undefined) base[key] = tool[key];
  if (tool.status !== 'streaming' && tool.toolInput !== undefined) base.toolInput = tool.toolInput;
  return base;
}
/** Outbound votes carry nonce-bound host option IDs. A newer challenge cannot inherit an old prediction. */
export function canPredict(chat: Chat, action: Json): boolean {
  if (action.type === 'chat/inputCompleted') return chat.interactivity === 'full' && chat.activeTurn?.responseParts.some((p: Json) => p.kind === 'inputRequest' && p.request?.id === action.requestId && !p.response) === true;
  if (action.type !== 'chat/toolCallConfirmed') return true;
  const turn = chat.activeTurn;
  const tool = turn?.responseParts.find((p: Json) => p.kind === 'toolCall' && p.toolCall?.toolCallId === action.toolCallId)?.toolCall;
  return turn?.id === action.turnId && tool?.status === 'pending-confirmation' && tool.options?.some((o: Json) => o.id === action.selectedOptionId && ['approve', 'deny'].includes(o.kind) && (o.kind === 'approve') === action.approved) === true;
}
/** Match pinned SDK end_turn: terminal turns cannot retain unfinished tool calls. */
function terminalPart(part: Json): Json {
  if (part.kind !== 'toolCall' || !object(part.toolCall) || ['completed', 'cancelled'].includes(part.toolCall.status)) return part;
  const tool = part.toolCall;
  const cancelled: Json = { status: 'cancelled', toolCallId: tool.toolCallId, toolName: tool.toolName, displayName: tool.displayName, invocationMessage: tool.invocationMessage ?? '', reason: 'skipped' };
  for (const key of ['intention', 'contributor', '_meta']) if (tool[key] !== undefined) cancelled[key] = tool[key];
  // Streaming only has partialInput. Final toolInput exists from ready onwards.
  if (tool.status !== 'streaming' && tool.toolInput !== undefined) cancelled.toolInput = tool.toolInput;
  return { kind: 'toolCall', toolCall: cancelled };
}
function activityStatus(state: Chat, terminal?: number): number {
  const blocked = state.activeTurn?.responseParts.some((p: Json) => p.kind === 'toolCall' && ['pending-confirmation', 'pending-result-confirmation', 'auth-required'].includes(p.toolCall?.status) || p.kind === 'inputRequest' && !p.response);
  return ((state.status ?? 0) & ~31) | (terminal ?? (blocked ? 24 : state.activeTurn ? 8 : 1));
}
function refineContributor(current: Json | undefined, next: Json | undefined): Json | undefined {
  if (!next) return current;
  if (current?.kind === 'client') return next.kind === 'client' && next.clientId === current.clientId ? next : current;
  return next.kind === 'client' ? current : next;
}
export function reduceChat(previous: Chat, action: Json): Chat {
  const state = structuredClone(previous);
  const turn = state.activeTurn;
  const type = action.type;
  if (type === 'chat/turnsLoaded') {
    if (!Array.isArray(action.turns) || action.turns.length > 256 || action.turns.some((t: unknown) => !validTurn(t)) || !cursor(action.turnsNextCursor) || new Set(action.turns.map((t: Json) => t.id)).size !== action.turns.length) throw new Error('Niepoprawna strona historii AHP.');
    const existing = new Set(state.turns.map((t: Json) => t.id));
    state.turns = [...action.turns.filter((t: Json) => !existing.has(t.id)), ...state.turns];
    if (action.turnsNextCursor === undefined) delete state.turnsNextCursor; else state.turnsNextCursor = action.turnsNextCursor;
  } else if (type === 'chat/inputRequested' && turn) {
    const existing = turn.responseParts.find((p: Json) => p.kind === 'inputRequest' && !p.response && p.request.id === action.request.id);
    const request = structuredClone(action.request);
    if (existing) { if (request.answers === undefined && existing.request.answers !== undefined) request.answers = existing.request.answers; existing.request = request; }
    else turn.responseParts.push({ kind: 'inputRequest', request });
    state.status = activityStatus(state) & ~32;
  } else if (['chat/inputAnswerChanged', 'chat/inputCompleted'].includes(type) && turn) {
    const part = turn.responseParts.find((p: Json) => p.kind === 'inputRequest' && !p.response && p.request.id === action.requestId);
    if (part) {
      let answers = { ...part.request.answers };
      if (type === 'chat/inputAnswerChanged') { if (action.answer === undefined) delete answers[action.questionId]; else answers = { ...answers, [action.questionId]: structuredClone(action.answer) }; }
      else { answers = { ...answers, ...structuredClone(action.answers ?? {}) }; part.response = action.response; }
      if (Object.keys(answers).length) part.request.answers = answers; else delete part.request.answers;
      if (type === 'chat/inputCompleted') state.status = activityStatus(state);
    }
  } else if (type === 'chat/pendingMessageSet' && action.kind === 'queued') {
    const entries: Json[] = state.queuedMessages ?? [];
    const index = entries.findIndex((m) => m.id === action.id);
    const entry = { id: action.id, message: action.message };
    if (index >= 0) entries[index] = entry; else entries.push(entry);
    state.queuedMessages = entries;
  } else if (type === 'chat/pendingMessageRemoved' && action.kind === 'queued') {
    if (state.queuedMessages) {
      const entries = state.queuedMessages.filter((m: Json) => m.id !== action.id);
      if (entries.length !== state.queuedMessages.length) {
        if (entries.length) state.queuedMessages = entries; else delete state.queuedMessages;
      }
    }
  } else if (type === 'chat/queuedMessagesReordered' && state.queuedMessages) {
    const entries: Json[] = state.queuedMessages;
    const order: string[] = [...new Set<string>(action.order)];
    // Protocol docs preserve the original order of omitted messages. Pinned
    // Rust SDK sorts omitted IDs: an explicit known-divergence oracle covers it.
    state.queuedMessages = [...order.flatMap((id) => entries.filter((m) => m.id === id)), ...entries.filter((m) => !order.includes(m.id))];
  } else if (type === 'chat/turnStarted') {
    state.activeTurn = { id: action.turnId, message: action.message, startedAt: action.startedAt, responseParts: [] };
    state.status = activityStatus(state) & ~32;
    state.modifiedAt = action.startedAt;
    if (action.queuedMessageId !== undefined) {
      if (state.steeringMessage?.id === action.queuedMessageId) delete state.steeringMessage;
      if (state.queuedMessages) {
        state.queuedMessages = state.queuedMessages.filter((m: Json) => m.id !== action.queuedMessageId);
        if (!state.queuedMessages.length) delete state.queuedMessages;
      }
    }
  } else if (turn && turn.id === action.turnId) {
    const parts: Json[] = turn.responseParts;
    if (type === 'chat/responsePart') {
      if (!object(action.part) || typeof action.part.kind !== 'string') throw new Error('Niepoprawny fragment odpowiedzi.');
      if (action.part.kind !== 'error') parts.push(action.part);
    } else if (type === 'chat/delta' || type === 'chat/reasoning') {
      if (typeof action.content !== 'string') throw new Error('Niepoprawna zawartość AHP.');
      const part = parts.find((p) => p.id === action.partId);
      if (part?.kind === (type === 'chat/delta' ? 'markdown' : 'reasoning')) part.content += action.content;
    } else if (type === 'chat/toolCallStart') {
      const tool: Json = { status: 'streaming', toolCallId: action.toolCallId, toolName: action.toolName, displayName: action.displayName };
      for (const key of ['intention', 'contributor', '_meta']) if (action[key] !== undefined) tool[key] = action[key];
      parts.push({ kind: 'toolCall', toolCall: tool });
    } else if (type?.startsWith('chat/toolCall')) {
      const part = parts.find((p) => p.kind === 'toolCall' && p.toolCall?.toolCallId === action.toolCallId);
      const tool = part?.toolCall;
      if (!tool) return state;
      if (type === 'chat/toolCallDelta' && tool.status === 'streaming') {
        if (action.content !== undefined) tool.partialInput = (tool.partialInput ?? '') + action.content;
        if (action.invocationMessage !== undefined) tool.invocationMessage = action.invocationMessage;
        if (action._meta !== undefined) tool._meta = action._meta;
      }
      if (type === 'chat/toolCallReady' && ['streaming', 'running', 'pending-confirmation'].includes(tool.status)) {
        const next: Json = { ...toolBase(tool), invocationMessage: action.invocationMessage, status: action.confirmed !== undefined ? 'running' : 'pending-confirmation' };
        for (const key of ['intention', 'toolInput', '_meta']) if (action[key] !== undefined) next[key] = action[key];
        const contributor = refineContributor(tool.contributor, action.contributor);
        if (contributor !== undefined) next.contributor = contributor;
        if (action.confirmed !== undefined) next.confirmed = action.confirmed;
        else for (const key of ['confirmationTitle', 'riskAssessment', 'edits', 'editable', 'options']) { const value = action[key] ?? (tool.status === 'pending-confirmation' ? tool[key] : undefined); if (value !== undefined) next[key] = value; }
        part!.toolCall = next;
      }
      if (type === 'chat/toolCallConfirmed' && tool.status === 'pending-confirmation') {
        const next: Json = { ...toolBase(tool), invocationMessage: tool.invocationMessage, status: action.approved ? 'running' : 'cancelled' };
        if (action._meta !== undefined) next._meta = action._meta;
        const selected = tool.options?.find((o: Json) => o.id === action.selectedOptionId);
        if (selected) next.selectedOption = selected;
        if (action.approved) next.confirmed = action.confirmed ?? 'not-needed';
        else { next.reason = action.reason ?? 'denied'; if (action.reasonMessage !== undefined) next.reasonMessage = action.reasonMessage; if (action.userSuggestion !== undefined) next.userSuggestion = action.userSuggestion; }
        part!.toolCall = next;
      }
      if (type === 'chat/toolCallContentChanged' && tool.status === 'running') {
        tool.content = action.content;
        if (action._meta !== undefined) tool._meta = action._meta;
      }
      if (type === 'chat/toolCallComplete' && ['running', 'pending-confirmation', 'auth-required'].includes(tool.status) && !(tool.status === 'auth-required' && action.result.success)) {
        const fromAuth = tool.status === 'auth-required';
        const next: Json = { ...toolBase(tool), invocationMessage: tool.invocationMessage, status: action.requiresResultConfirmation && !fromAuth ? 'pending-result-confirmation' : 'completed', confirmed: tool.status === 'pending-confirmation' ? 'not-needed' : tool.confirmed, success: action.result.success };
        if (tool.status !== 'pending-confirmation' && tool.selectedOption !== undefined) next.selectedOption = tool.selectedOption;
        for (const key of ['pastTenseMessage', 'content', 'structuredContent', 'error']) if (action.result[key] !== undefined) next[key] = action.result[key];
        if (fromAuth && action.result.content === undefined && tool.content !== undefined) next.content = tool.content;
        if (action._meta !== undefined) next._meta = action._meta;
        part!.toolCall = next;
      }
      if (['chat/toolCallReady', 'chat/toolCallConfirmed', 'chat/toolCallComplete'].includes(type)) state.status = activityStatus(state);
    } else if (['chat/turnComplete', 'chat/turnCancelled', 'chat/error'].includes(type)) {
      const duration = Math.max(0, action.duration);
      const end = Date.parse(turn.startedAt) + duration;
      if (Number.isFinite(end)) state.modifiedAt = new Date(end).toISOString();
      if (type === 'chat/error') parts.push(action.part);
      state.turns.push({ ...turn, responseParts: parts.map(terminalPart), state: type === 'chat/error' ? 'error' : type === 'chat/turnCancelled' ? 'cancelled' : 'complete', duration });
      delete state.activeTurn;
      state.status = activityStatus(state, type === 'chat/error' ? 2 : undefined);
    }
  }
  if (state.turns.length > 256) throw new Error('Transkrypt przekracza limit widoku.');
  return state;
}
