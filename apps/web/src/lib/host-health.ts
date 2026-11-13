import { decodeHost, hostPath } from './hosts.ts';
import type { Host } from './hosts.ts';
import { validateEndpoint } from './transport.ts';
export const hostHealthProtocol = 'dunlin.host-health.v1';
export type HostHealth = {
  host: Host;
  connection: null | {
    protocol_version: 1 | 2 | 3 | 4 | 5;
    heartbeat_sequence: number;
    heartbeat_age_ms: number;
    lease_remaining_ms: number;
  };
  runtime: {
    one_shot_reservations: number;
    session_reservations: number;
    active_sessions: number;
    active_turns: number;
    uncertain_reservations: number;
    unconfirmed_acknowledgements: number;
  };
  controller_boot_id: string;
  observed_at_ms: number;
  observational: true;
};
type Event =
  | { type: 'snapshot'; stream_id: string; sequence: number; health: HostHealth }
  | { type: 'heartbeat'; stream_id: string }
  | { type: 'unavailable'; stream_id: string };
const object = (v: unknown): v is Record<string, unknown> => !!v && typeof v === 'object' && !Array.isArray(v);
const only = (v: Record<string, unknown>, keys: string[]) => Object.keys(v).every((key) => keys.includes(key));
const integer = (v: unknown, min = 0, max = Number.MAX_SAFE_INTEGER): v is number =>
  Number.isSafeInteger(v) && Number(v) >= min && Number(v) <= max;
const id = (v: unknown): v is string => typeof v === 'string' && /^[A-Za-z0-9_-]{1,128}$/.test(v);
export function decodeHostHealth(value: unknown, workspace: string, host: string): HostHealth {
  hostPath(workspace, host);
  if (!object(value) || !only(value, ['host', 'connection', 'runtime', 'controller_boot_id', 'observed_at_ms', 'observational'])
    || !object(value.host) || !only(value.host, ['id', 'workspace_id', 'definition', 'status', 'revision', 'enrollment_epoch',
      'credential_revision', 'capability_revision', 'drain', 'capabilities', 'last_seen', 'created_at', 'updated_at', 'network_verification', 'execution_ready'])
    || value.host.id !== host || !id(value.controller_boot_id) || value.observational !== true
    || !integer(value.observed_at_ms, 0, 8_640_000_000_000_000)
    || (value.host.capability_revision !== undefined && !integer(value.host.capability_revision))
    || !object(value.runtime) || !only(value.runtime, ['one_shot_reservations', 'session_reservations', 'active_sessions',
      'active_turns', 'uncertain_reservations', 'unconfirmed_acknowledgements'])) throw new Error('Niepoprawna obserwacja hosta.');
  const runtime = value.runtime;
  for (const key of ['one_shot_reservations', 'session_reservations', 'active_sessions', 'active_turns', 'uncertain_reservations', 'unconfirmed_acknowledgements'])
    if (!integer(runtime[key], 0, 32768)) throw new Error('Niepoprawne liczniki runtime.');
  if (Number(runtime.active_sessions) > Number(runtime.session_reservations)
    || Number(runtime.active_turns) > Number(runtime.active_sessions)
    || Number(runtime.uncertain_reservations) > Number(runtime.one_shot_reservations) + Number(runtime.session_reservations))
    throw new Error('Niespójne rezerwacje runtime.');
  let connection: HostHealth['connection'] = null;
  if (value.connection !== null) {
    const c = value.connection;
    if (!object(c) || !only(c, ['protocol_version', 'heartbeat_sequence', 'heartbeat_age_ms', 'lease_remaining_ms'])
      || !integer(c.protocol_version, 1, 5) || !integer(c.heartbeat_sequence)
      || !integer(c.heartbeat_age_ms, 0, 9999) || !integer(c.lease_remaining_ms, 1, 10000)
      || c.heartbeat_age_ms + c.lease_remaining_ms > 10000) throw new Error('Niepoprawna obserwacja kanału.');
    connection = { protocol_version: c.protocol_version as 1 | 2 | 3 | 4 | 5, heartbeat_sequence: c.heartbeat_sequence,
      heartbeat_age_ms: c.heartbeat_age_ms, lease_remaining_ms: c.lease_remaining_ms };
  }
  return { host: decodeHost(value.host, workspace), connection,
    runtime: { one_shot_reservations: Number(runtime.one_shot_reservations), session_reservations: Number(runtime.session_reservations),
      active_sessions: Number(runtime.active_sessions), active_turns: Number(runtime.active_turns),
      uncertain_reservations: Number(runtime.uncertain_reservations), unconfirmed_acknowledgements: Number(runtime.unconfirmed_acknowledgements) },
    controller_boot_id: value.controller_boot_id, observed_at_ms: value.observed_at_ms, observational: true };
}
export function decodeHostHealthEvent(data: string, workspace: string, host: string): Event {
  if (new TextEncoder().encode(data).length > 16 * 1024) throw new Error('Przekroczony limit obserwacji.');
  const value: unknown = JSON.parse(data);
  if (!object(value) || value.protocol_version !== 1 || !id(value.stream_id)) throw new Error('Niepoprawny protokół obserwacji.');
  if (value.type === 'snapshot' && only(value, ['type', 'protocol_version', 'stream_id', 'sequence', 'health']) && integer(value.sequence, 1))
    return { type: 'snapshot', stream_id: value.stream_id, sequence: value.sequence, health: decodeHostHealth(value.health, workspace, host) };
  if ((value.type === 'heartbeat' || value.type === 'unavailable') && only(value, ['type', 'protocol_version', 'stream_id']))
    return { type: value.type, stream_id: value.stream_id };
  throw new Error('Nieobsługiwany komunikat obserwacji.');
}
export type HostHealthState = {
  status: 'idle' | 'connecting' | 'live' | 'offline' | 'forbidden' | 'stale' | 'error';
  health: HostHealth | null;
  error: string;
};
const idle = (): HostHealthState => ({ status: 'idle', health: null, error: '' });
/** Explicit read-only starts. Scope, stream order and local freshness are fenced;
 * no REST polling, mutation replay, credentials in frames, or browser storage. */
export class HostHealthStream {
  #socket: WebSocket | null = null;
  #stopped = true;
  #stream: string | null = null;
  #sequence = 0;
  #watchdog: ReturnType<typeof setTimeout> | null = null;
  #lease: ReturnType<typeof setTimeout> | null = null;
  #state = idle();
  #url: string;
  #workspace: string;
  #host: string;
  #changed: (state: HostHealthState) => void;
  #make: (url: string, protocol: string) => WebSocket;
  constructor(origin: string, workspace: string, host: string, changed: (state: HostHealthState) => void,
    make: (url: string, protocol: string) => WebSocket = (url, protocol) => new WebSocket(url, protocol)) {
    const url = validateEndpoint(origin);
    url.protocol = url.protocol === 'https:' || url.protocol === 'wss:' ? 'wss:' : 'ws:';
    url.pathname = `${hostPath(workspace, host)}/health/live`;
    this.#url = url.toString(); this.#workspace = workspace; this.#host = host; this.#changed = changed; this.#make = make;
  }
  get state(): HostHealthState { return this.#state; }
  #publish(state: HostHealthState) { this.#state = state; this.#changed(state); }
  #clearTimers() {
    if (this.#watchdog !== null) clearTimeout(this.#watchdog);
    if (this.#lease !== null) clearTimeout(this.#lease);
    this.#watchdog = null; this.#lease = null;
  }
  stop(): void {
    this.#stopped = true;
    const socket = this.#socket; this.#socket = null;
    this.#clearTimers(); this.#stream = null; this.#sequence = 0; this.#state = idle(); socket?.close();
  }
  #fail(status: HostHealthState['status'], error: string) {
    this.stop(); this.#publish({ status, health: null, error });
  }
  #fresh() {
    if (this.#watchdog !== null) clearTimeout(this.#watchdog);
    this.#watchdog = setTimeout(() => this.#fail('stale', 'Podgląd utracił aktualność. Połącz ponownie.'), 12000);
  }
  start(): void {
    this.stop(); this.#stopped = false;
    this.#publish({ status: 'connecting', health: null, error: '' });
    let socket: WebSocket;
    try { socket = this.#make(this.#url, hostHealthProtocol); }
    catch { this.#fail('error', 'Nie udało się otworzyć podglądu hosta.'); return; }
    this.#socket = socket; this.#fresh();
    const current = () => !this.#stopped && this.#socket === socket;
    socket.onmessage = (message) => {
      if (!current()) return;
      try {
        if (typeof message.data !== 'string') throw new Error('Unsupported frame');
        const event = decodeHostHealthEvent(message.data, this.#workspace, this.#host);
        if (this.#stream !== null && event.stream_id !== this.#stream) throw new Error('Wrong stream');
        if (event.type === 'unavailable') { this.#fail('forbidden', 'Podgląd niedostępny. Sprawdź bieżący dostęp do hosta.'); return; }
        if (event.type === 'heartbeat') {
          if (this.#stream === null) throw new Error('Missing scoped snapshot');
          this.#fresh(); return; // A keepalive never renews the worker observation.
        }
        if (event.sequence !== this.#sequence + 1
          || (this.#state.health !== null && event.health.controller_boot_id !== this.#state.health.controller_boot_id)) throw new Error('Observation order changed');
        this.#stream = event.stream_id; this.#sequence = event.sequence;
        if (this.#lease !== null) clearTimeout(this.#lease);
        this.#lease = event.health.connection === null ? null : setTimeout(() => {
          if (current()) this.#fail('stale', 'Obserwacja połączenia wygasła. Połącz ponownie.');
        }, event.health.connection.lease_remaining_ms);
        this.#fresh(); this.#publish({ status: 'live', health: event.health, error: '' });
      } catch { this.#fail('error', 'Odebrano niespójny podgląd hosta. Połącz ponownie.'); }
    };
    socket.onclose = () => { if (current()) this.#fail('offline', 'Połączenie podglądu zostało zamknięte.'); };
    socket.onerror = () => { if (current()) this.#fail('error', 'Nie udało się odebrać podglądu hosta.'); };
  }
}
