import { hostPath } from './hosts.ts';
import { teamRequest, TeamRequestError } from './teams.ts';

export type HostChannelDiagnostic = {
  channel: null | {
    lease: {
      host_id: string;
      workspace_id: string;
      channel_id: string;
      controller_boot_id: string;
      worker_boot_id: string;
      epoch: number;
      expires_at_ms: number;
      last_sequence: number;
    };
    phase: 'active' | 'offline';
  };
  current_controller_boot_id: string;
  reported_state_only: true;
};
function object(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === 'object' && !Array.isArray(value);
}
function only(value: Record<string, unknown>, keys: string[]): boolean {
  return Object.keys(value).every((key) => keys.includes(key));
}
function id(value: unknown): value is string {
  return typeof value === 'string' && /^[A-Za-z0-9_-]{1,128}$/.test(value);
}
function integer(value: unknown, minimum = 0): value is number {
  return Number.isSafeInteger(value) && Number(value) >= minimum;
}
export function decodeHostChannel(value: unknown, workspace: string, host: string): HostChannelDiagnostic {
  hostPath(workspace, host);
  if (
    !object(value) ||
    !only(value, ['channel', 'current_controller_boot_id', 'reported_state_only']) ||
    !id(value.current_controller_boot_id) ||
    value.reported_state_only !== true
  ) {
    throw new Error('Niepoprawna odpowiedź diagnostyki kanału hosta.');
  }
  if (value.channel === null)
    return {
      channel: null,
      current_controller_boot_id: value.current_controller_boot_id,
      reported_state_only: true,
    };
  const channel = value.channel;
  if (
    !object(channel) ||
    !only(channel, ['lease', 'phase']) ||
    !['active', 'offline'].includes(String(channel.phase)) ||
    typeof channel.phase !== 'string' ||
    !object(channel.lease)
  )
    throw new Error('Niepoprawny zapis kanału hosta.');
  const lease = channel.lease;
  if (
    !only(lease, [
      'host_id',
      'workspace_id',
      'channel_id',
      'controller_boot_id',
      'worker_boot_id',
      'epoch',
      'expires_at_ms',
      'last_sequence',
    ]) ||
    lease.host_id !== host ||
    lease.workspace_id !== workspace ||
    !id(lease.channel_id) ||
    !id(lease.controller_boot_id) ||
    !id(lease.worker_boot_id) ||
    !integer(lease.epoch, 1) ||
    !integer(lease.expires_at_ms) ||
    lease.expires_at_ms > 8_640_000_000_000_000 ||
    !integer(lease.last_sequence)
  ) {
    throw new Error('Niespójny zakres lub lease diagnostyki hosta.');
  }
  // Rebuild from checked fields: no unknown secret/authority fields enter state.
  return {
    channel: {
      phase: channel.phase as 'active' | 'offline',
      lease: {
        host_id: host,
        workspace_id: workspace,
        channel_id: lease.channel_id,
        controller_boot_id: lease.controller_boot_id,
        worker_boot_id: lease.worker_boot_id,
        epoch: lease.epoch,
        expires_at_ms: lease.expires_at_ms,
        last_sequence: lease.last_sequence,
      },
    },
    current_controller_boot_id: value.current_controller_boot_id,
    reported_state_only: true,
  };
}
export function channelWarnings(
  value: HostChannelDiagnostic,
  readAt: number,
): { differentBoot: boolean; expiredAtRead: boolean } {
  return {
    differentBoot:
      !!value.channel && value.channel.lease.controller_boot_id !== value.current_controller_boot_id,
    expiredAtRead: !!value.channel && value.channel.lease.expires_at_ms <= readAt,
  };
}
export type HostChannelState = {
  status: 'idle' | 'loading' | 'loaded' | 'error' | 'forbidden';
  diagnostic: HostChannelDiagnostic | null;
  readAt: number | null;
  error: string;
};
const idle = (): HostChannelState => ({ status: 'idle', diagnostic: null, readAt: null, error: '' });
/** Explicit reads only. Scope is immutable; stop/invalidate fence even fetch
 * implementations which return a late response after AbortSignal cancellation. */
export class HostChannelReader {
  #generation = 0;
  #stopped = false;
  #controller: AbortController | null = null;
  #state: HostChannelState = idle();
  #workspace: string;
  #host: string;
  #changed: (state: HostChannelState) => void;
  #forbidden: () => void;
  constructor(
    workspace: string,
    host: string,
    changed: (state: HostChannelState) => void,
    forbidden: () => void,
  ) {
    hostPath(workspace, host);
    this.#workspace = workspace;
    this.#host = host;
    this.#changed = changed;
    this.#forbidden = forbidden;
  }
  get state(): HostChannelState {
    return this.#state;
  }
  #publish(state: HostChannelState): void {
    this.#state = state;
    this.#changed(state);
  }
  invalidate(): void {
    this.#generation++;
    this.#controller?.abort();
    this.#controller = null;
    this.#publish(idle());
  }
  stop(): void {
    this.#stopped = true;
    this.#generation++;
    this.#controller?.abort();
    this.#controller = null;
    this.#state = idle();
  }
  async read(): Promise<void> {
    if (this.#stopped) return;
    this.#controller?.abort();
    const controller = new AbortController();
    this.#controller = controller;
    const generation = ++this.#generation;
    const current = () => !this.#stopped && this.#generation === generation;
    this.#publish({ status: 'loading', diagnostic: null, readAt: null, error: '' });
    const timer = setTimeout(() => {
      if (!current()) return;
      controller.abort();
      this.#generation++;
      this.#controller = null;
      this.#publish({
        status: 'error',
        diagnostic: null,
        readAt: null,
        error: 'Przekroczono czas odczytu diagnostyki. Możesz ponowić odczyt jawnie.',
      });
    }, 15_000);
    try {
      const value = await teamRequest(`${hostPath(this.#workspace, this.#host)}/channel`, controller.signal);
      if (!current() || controller.signal.aborted) return;
      const diagnostic = decodeHostChannel(value, this.#workspace, this.#host);
      this.#publish({ status: 'loaded', diagnostic, readAt: Date.now(), error: '' });
    } catch (error) {
      if (!current()) return;
      const denied = error instanceof TeamRequestError && [401, 403, 404].includes(error.status);
      this.#publish({
        status: denied ? 'forbidden' : 'error',
        diagnostic: null,
        readAt: null,
        error: denied
          ? 'Diagnostyka nie jest dostępna. Dostęp do hosta mógł zostać odwołany.'
          : 'Nie udało się odczytać diagnostyki kanału.',
      });
      if (denied) this.#forbidden();
    } finally {
      clearTimeout(timer);
      if (current()) this.#controller = null;
    }
  }
}
