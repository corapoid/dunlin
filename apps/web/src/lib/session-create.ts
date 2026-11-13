/** Browser-only creation intents. Callers must supply an approved profile;
 * this module discovers neither models nor modes and never starts implicitly.
 * Intent identity/body are memory-only and survive an explicit uncertain retry.
 * Acceptance is historical: AHP/GET current state must gate subsequent prompts.
 */
export type SessionProfile = Readonly<{
  model: string;
  mode: string;
  max_seconds: number;
  max_output_bytes: number;
}>;
export type PublishedProfile = Readonly<{
  profile: Readonly<{ agent_id: string; version: number; digest: string }>;
  mcp_connections?: readonly Readonly<{provider_id:string;connection_id:string;revision:number}>[];
  target?: Readonly<{ kind: 'remote'; binding: Readonly<{ id: string; revision: number }> }>;
}>;
export type SessionCreationProfile = SessionProfile | PublishedProfile;
export type CreationScope = Readonly<{
  userId: string;
  deviceId: string;
  workspaceId: string;
  projectId: string;
}>;
export type SessionAcceptance = Readonly<{
  sessionId: string;
  workspaceId: string;
  projectId: string;
  ownerId: string;
  acceptedPhase: 'pending' | 'active' | 'interrupted';
  acceptedEpoch: number;
  acceptedSequence: number;
  replayed: boolean;
  sessionIsAcceptanceSnapshot: true;
}>;
export type CreationState = Readonly<{
  status: 'ready' | 'submitting' | 'uncertain' | 'blocked' | 'accepted';
  message: string;
}>;
function object(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === 'object' && !Array.isArray(value);
}
function id(value: unknown): value is string {
  return (
    typeof value === 'string' &&
    value.length > 0 &&
    value.length <= 128 &&
    !/[\u0000-\u001f\u007f]/.test(value)
  );
}
function path(scope: CreationScope): string {
  if (
    !id(scope.userId) ||
    !id(scope.deviceId) ||
    !/^[A-Za-z0-9_-]{1,128}$/.test(scope.workspaceId) ||
    !/^[A-Za-z0-9_-]{1,128}$/.test(scope.projectId)
  )
    throw new Error('Niepoprawny zakres utworzenia sesji.');
  return `/v1/web/workspaces/${scope.workspaceId}/projects/${scope.projectId}/interactive-sessions`;
}
function profile(value: SessionCreationProfile): SessionCreationProfile {
  if ('profile' in value) {
    const p = value.profile;
    if (
      !/^[A-Za-z0-9_-]{1,128}$/.test(p.agent_id) ||
      !Number.isSafeInteger(p.version) ||
      p.version < 1 ||
      !/^[a-f0-9]{64}$/.test(p.digest)
    )
      throw new Error('Wymagany jest opublikowany, przypięty profil agenta.');
    const target=value.target;
    if (target !== undefined && (!object(target) || target.kind !== 'remote'
      || Object.keys(target).some(key => !['kind','binding'].includes(key))
      || !object(target.binding) || Object.keys(target.binding).some(key => !['id','revision'].includes(key))
      || typeof target.binding.id !== 'string' || !/^[A-Za-z0-9_-]{1,128}$/.test(target.binding.id)
      || !Number.isSafeInteger(target.binding.revision) || target.binding.revision < 1))
      throw new Error('Wymagane jest aktualne powiązanie zdalnego celu i jego rewizja.');
    const connections=value.mcp_connections;
    if(connections!==undefined && (!Array.isArray(connections)||connections.length>8
      ||connections.some(c=>!object(c)||Object.keys(c).some(k=>!['provider_id','connection_id','revision'].includes(k))
        ||typeof c.provider_id!=='string'||!/^[A-Za-z0-9_-]{1,64}$/.test(c.provider_id)
        ||!id(c.connection_id)||typeof c.revision!=='number'||!Number.isSafeInteger(c.revision)||c.revision<1)
      ||new Set(connections.map(c=>c.provider_id)).size!==connections.length))
      throw new Error('Wymagany jest jawny wybór prywatnych połączeń MCP dla sesji.');
    return Object.freeze({
      profile: Object.freeze({ agent_id: p.agent_id, version: p.version, digest: p.digest }),
      ...(!connections?.length?{}:{mcp_connections:Object.freeze(connections.map(c=>Object.freeze({provider_id:c.provider_id,connection_id:c.connection_id,revision:c.revision})))}),
      ...(target === undefined ? {} : {target:Object.freeze({kind:'remote' as const,binding:Object.freeze({id:target.binding.id,revision:target.binding.revision})})}),
    });
  }
  if ('target' in value) throw new Error('Zdalny cel wymaga opublikowanego profilu agenta.');
  const text = (v: unknown, limit: number) =>
    typeof v === 'string' &&
    !!v.trim() &&
    !/[\u0000-\u001f\u007f]/.test(v) &&
    new TextEncoder().encode(v).length <= limit;
  if (
    !text(value.model, 256) ||
    !text(value.mode, 128) ||
    !Number.isSafeInteger(value.max_seconds) ||
    value.max_seconds < 1 ||
    value.max_seconds > 1800 ||
    !Number.isSafeInteger(value.max_output_bytes) ||
    value.max_output_bytes < 1024 ||
    value.max_output_bytes > 4 * 1024 * 1024
  )
    throw new Error('Wymagany jest jawny zatwierdzony profil modelu, trybu i limitów.');
  return Object.freeze({
    model: value.model,
    mode: value.mode,
    max_seconds: value.max_seconds,
    max_output_bytes: value.max_output_bytes,
  });
}
export class SessionCreationError extends Error {
  readonly kind: 'uncertain' | 'rejected' | 'identity';
  readonly status?: number;
  constructor(kind: 'uncertain' | 'rejected' | 'identity', message: string, status?: number) {
    super(message);
    this.kind = kind;
    this.status = status;
  }
}
function refusal(status: number): SessionCreationError {
  return status === 429 || status === 408 || status >= 500
    ? new SessionCreationError(
        'uncertain',
        'Nie potwierdzono utworzenia sesji. Możesz jawnie ponowić ten sam zamiar z tym samym kluczem.',
        status,
      )
    : new SessionCreationError(
        'rejected',
        status === 409
          ? 'Konflikt żądania lub uprawnień. Sprawdź istniejące sesje przed rozpoczęciem kolejnego zamiaru.'
          : 'Serwer odrzucił utworzenie sesji. Sprawdź bieżące uprawnienia i profil.',
        status,
      );
}
function acceptance(value: unknown, scope: CreationScope, replayed: string | null): SessionAcceptance {
  if (!object(value) || !object(value.session) || !object(value.admission))
    throw new Error('Niepoprawne potwierdzenie utworzenia sesji.');
  const s = value.session,
    a = value.admission;
  if (
    !id(s.id) ||
    !/^[A-Za-z0-9_-]{1,128}$/.test(s.id) ||
    s.workspace !== scope.workspaceId ||
    s.project !== scope.projectId ||
    s.owner !== scope.userId ||
    typeof s.phase !== 'string' ||
    !['pending', 'active', 'interrupted'].includes(s.phase) ||
    !Number.isSafeInteger(s.epoch) ||
    Number(s.epoch) < 0 ||
    !Number.isSafeInteger(s.sequence) ||
    Number(s.sequence) < 0 ||
    typeof a.replayed !== 'boolean' ||
    a.session_is_acceptance_snapshot !== true ||
    !['true', 'false'].includes(replayed ?? '') ||
    String(a.replayed) !== replayed
  )
    throw new Error('Niespójne historyczne potwierdzenie utworzenia sesji.');
  return Object.freeze({
    sessionId: s.id,
    workspaceId: scope.workspaceId,
    projectId: scope.projectId,
    ownerId: scope.userId,
    acceptedPhase: s.phase as SessionAcceptance['acceptedPhase'],
    acceptedEpoch: Number(s.epoch),
    acceptedSequence: Number(s.sequence),
    replayed: a.replayed,
    sessionIsAcceptanceSnapshot: true,
  });
}
export class SessionCreationIntent {
  #command: Readonly<{ scope: CreationScope; path: string; body: string; key: string }>;
  #state: CreationState = Object.freeze({ status: 'ready', message: '' });
  #accepted: SessionAcceptance | null = null;
  #blocked: SessionCreationError | null = null;
  #inFlight: Promise<SessionAcceptance> | null = null;
  constructor(scope: CreationScope, approvedProfile: SessionCreationProfile) {
    const endpoint = path(scope),
      selected = profile(approvedProfile);
    this.#command = Object.freeze({
      scope: Object.freeze({ ...scope }),
      path: endpoint,
      body: JSON.stringify(selected),
      key: crypto.randomUUID(),
    });
  }
  get key(): string {
    return this.#command.key;
  }
  get state(): CreationState {
    return this.#state;
  }
  get accepted(): SessionAcceptance | null {
    return this.#accepted;
  }
  submit(signal?: AbortSignal): Promise<SessionAcceptance> {
    if (this.#accepted) return Promise.resolve(this.#accepted);
    if (this.#blocked) return Promise.reject(this.#blocked);
    if (this.#inFlight) return this.#inFlight;
    const attempt = this.#perform(signal).finally(() => {
      if (this.#inFlight === attempt) this.#inFlight = null;
    });
    this.#inFlight = attempt;
    return attempt;
  }
  async #perform(signal?: AbortSignal): Promise<SessionAcceptance> {
    this.#state = Object.freeze({ status: 'submitting', message: '' });
    const controller = new AbortController(),
      cancel = () => controller.abort();
    if (signal?.aborted) controller.abort();
    else signal?.addEventListener('abort', cancel, { once: true });
    const timer = setTimeout(cancel, 15_000);
    const options = {
      credentials: 'same-origin',
      cache: 'no-store',
      redirect: 'error',
      signal: controller.signal,
    } as const;
    try {
      // Preflight improves feedback. The pinned headers below enforce identity
      // atomically server-side even if the cookie changes after this GET.
      const current = await fetch('/v1/web/session', options);
      if (!current.ok) throw refusal(current.status);
      const identity: unknown = await current.json();
      if (
        !object(identity) ||
        !object(identity.user) ||
        identity.user.id !== this.#command.scope.userId ||
        identity.device_id !== this.#command.scope.deviceId
      )
        throw new SessionCreationError(
          'identity',
          'Tożsamość przeglądarki zmieniła się. Sprawdź historię pierwotnego użytkownika przed kolejną próbą.',
        );
      const response = await fetch(this.#command.path, {
        ...options,
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'Idempotency-Key': this.#command.key,
          'X-Dunlin-Expected-User': this.#command.scope.userId,
          'X-Dunlin-Expected-Device': this.#command.scope.deviceId,
        },
        body: this.#command.body,
      });
      if (response.status !== 202) throw refusal(response.status);
      const confirmed = acceptance(
        await response.json(),
        this.#command.scope,
        response.headers.get('Idempotency-Replayed'),
      );
      this.#accepted = confirmed;
      this.#state = Object.freeze({
        status: 'accepted',
        message: 'Przyjęto utworzenie sesji. Bieżący stan sprawdź w AHP; to potwierdzenie jest historyczne.',
      });
      return confirmed;
    } catch (cause) {
      const failure =
        cause instanceof SessionCreationError
          ? cause
          : new SessionCreationError(
              'uncertain',
              'Wynik utworzenia sesji jest nieznany. Ponów wyłącznie ten sam zamiar; nie twórz automatycznie drugiej sesji.',
            );
      if (failure.kind !== 'uncertain') this.#blocked = failure;
      this.#state = Object.freeze({
        status: failure.kind === 'uncertain' ? 'uncertain' : 'blocked',
        message: failure.message,
      });
      throw failure;
    } finally {
      clearTimeout(timer);
      signal?.removeEventListener('abort', cancel);
    }
  }
}
/** One creation workflow per authenticated browser identity. No implicit reset
 * after unknown, rejection or recovery: resolve the original acceptance first. */
export class SessionCreationClient {
  #scope: Readonly<Pick<CreationScope, 'userId' | 'deviceId'>>;
  #current: SessionCreationIntent | null = null;
  constructor(identity: Pick<CreationScope, 'userId' | 'deviceId'>) {
    if (!id(identity.userId) || !id(identity.deviceId))
      throw new Error('Niepoprawna tożsamość przeglądarki.');
    this.#scope = Object.freeze({ ...identity });
  }
  get current(): SessionCreationIntent | null {
    return this.#current;
  }
  begin(
    workspaceId: string,
    projectId: string,
    approvedProfile: SessionCreationProfile,
  ): SessionCreationIntent {
    if (this.#current && this.#current.state.status !== 'accepted')
      throw new SessionCreationError(
        'uncertain',
        'Najpierw potwierdź wynik poprzedniego zamiaru utworzenia sesji.',
      );
    this.#current = new SessionCreationIntent({ ...this.#scope, workspaceId, projectId }, approvedProfile);
    return this.#current;
  }
}
