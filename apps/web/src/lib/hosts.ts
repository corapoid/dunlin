import { teamRequest } from './teams.ts';
export const hostHarnesses = ['opencode', 'codex', 'claude'] as const;
export type HostDefinition = {
  name: string;
  node_id: string;
  serve_origin: string;
  capability_policy: { max_parallel: number; harnesses: string[] };
};
export type HostCapabilities = {
  protocol_version: 1;
  version: string;
  os: string;
  architecture: string;
  harnesses: string[];
  sandbox: 'none' | 'linux_bubblewrap' | 'macos_sandbox_exec';
  max_parallel: number;
};
export type Host = {
  id: string;
  workspace_id: string;
  definition: HostDefinition;
  status: 'pending' | 'claimed' | 'approved' | 'revoked';
  revision: number;
  enrollment_epoch: number;
  credential_revision: number;
  drain: boolean;
  capabilities: HostCapabilities | null;
  last_seen: number | null;
  created_at: number;
  updated_at: number;
  network_verification: 'unverified';
  execution_ready: false;
};
export type HostEnrollment = { host: Host; enrollment: { ticket: string; expires_at: number } };
function object(v: unknown): v is Record<string, unknown> {
  return !!v && typeof v === 'object' && !Array.isArray(v);
}
function token(v: unknown): v is string {
  return typeof v === 'string' && /^[A-Za-z0-9_-]{1,128}$/.test(v);
}
function text(v: unknown, max: number): v is string {
  return (
    typeof v === 'string' && !!v.trim() && new TextEncoder().encode(v).length <= max && !/\p{Cc}/u.test(v)
  );
}
function integer(v: unknown, min = 0): v is number {
  return Number.isSafeInteger(v) && Number(v) >= min;
}
function harnesses(v: unknown): v is string[] {
  return (
    Array.isArray(v) &&
    v.length <= 3 &&
    v.every((h) => hostHarnesses.includes(h)) &&
    new Set(v).size === v.length
  );
}
export function hostPath(workspace: string, host?: string): string {
  if (!token(workspace) || (host !== undefined && !token(host))) throw new Error('Niepoprawny zakres hosta.');
  return `/v1/web/workspaces/${workspace}/hosts${host ? `/${host}` : ''}`;
}
export function decodeHostDefinition(v: unknown): HostDefinition {
  if (
    !object(v) ||
    Object.keys(v).some((k) => !['name', 'node_id', 'serve_origin', 'capability_policy'].includes(k)) ||
    !text(v.name, 128) ||
    !token(v.node_id) ||
    typeof v.serve_origin !== 'string' ||
    !object(v.capability_policy) ||
    Object.keys(v.capability_policy).some((k) => !['max_parallel', 'harnesses'].includes(k)) ||
    !integer(v.capability_policy.max_parallel, 1) ||
    v.capability_policy.max_parallel > 32 ||
    !harnesses(v.capability_policy.harnesses)
  )
    throw new Error('Niepoprawna definicja lub polityka hosta.');
  let url: URL;
  try {
    url = new URL(v.serve_origin);
  } catch {
    throw new Error('Wymagany jest kanoniczny origin HTTPS hosta Tailscale.');
  }
  if (
    url.hostname.length > 253 ||
    url.hostname.split('.').some((label) => label.length > 63) ||
    url.protocol !== 'https:' ||
    !url.hostname.endsWith('.ts.net') ||
    url.username ||
    url.password ||
    url.port ||
    url.origin !== v.serve_origin ||
    !/^(?:[a-z0-9](?:[a-z0-9-]*[a-z0-9])?\.)+ts\.net$/.test(url.hostname)
  )
    throw new Error('Wymagany jest kanoniczny origin HTTPS .ts.net bez portu, ścieżki i parametrów.');
  return structuredClone(v) as HostDefinition;
}
function decodeCapabilities(v: unknown): HostCapabilities {
  if (
    !object(v) ||
    Object.keys(v).some(
      (k) =>
        ![
          'protocol_version',
          'version',
          'os',
          'architecture',
          'harnesses',
          'sandbox',
          'max_parallel',
        ].includes(k),
    ) ||
    v.protocol_version !== 1 ||
    !text(v.version, 128) ||
    !text(v.os, 64) ||
    !text(v.architecture, 64) ||
    !harnesses(v.harnesses) ||
    typeof v.sandbox !== 'string' ||
    !['none', 'linux_bubblewrap', 'macos_sandbox_exec'].includes(v.sandbox) ||
    !integer(v.max_parallel, 1) ||
    v.max_parallel > 32
  )
    throw new Error('Niepoprawny raport możliwości hosta.');
  return structuredClone(v) as HostCapabilities;
}
export function decodeHost(v: unknown, workspace: string): Host {
  hostPath(workspace);
  if (
    !object(v) ||
    !token(v.id) ||
    v.workspace_id !== workspace ||
    typeof v.status !== 'string' ||
    !['pending', 'claimed', 'approved', 'revoked'].includes(v.status) ||
    !integer(v.revision, 1) ||
    !integer(v.enrollment_epoch) ||
    !integer(v.credential_revision) ||
    typeof v.drain !== 'boolean' ||
    (v.last_seen !== null && !integer(v.last_seen)) ||
    !integer(v.created_at) ||
    !integer(v.updated_at) ||
    v.network_verification !== 'unverified' ||
    v.execution_ready !== false
  )
    throw new Error('Niespójny zakres lub stan rejestru hostów.');
  return {
    ...v,
    definition: decodeHostDefinition(v.definition),
    capabilities: v.capabilities === null ? null : decodeCapabilities(v.capabilities),
  } as Host;
}
export function decodeHosts(v: unknown, workspace: string): Host[] {
  if (!object(v) || !Array.isArray(v.hosts) || v.hosts.length > 256)
    throw new Error('Niepoprawny katalog hostów.');
  const hosts = v.hosts.map((h) => decodeHost(h, workspace));
  if (new Set(hosts.map((h) => h.id)).size !== hosts.length) throw new Error('Powtórzony host.');
  return hosts;
}
export function decodeEnrollment(v: unknown, workspace: string): HostEnrollment {
  if (
    !object(v) ||
    !object(v.enrollment) ||
    !text(v.enrollment.ticket, 512) ||
    !integer(v.enrollment.expires_at, 1)
  )
    throw new Error('Niepoprawny bilet rejestracyjny.');
  return {
    host: decodeHost(v.host, workspace),
    enrollment: { ticket: v.enrollment.ticket, expires_at: v.enrollment.expires_at },
  };
}
export async function readHosts(workspace: string, signal?: AbortSignal): Promise<Host[]> {
  return decodeHosts(await teamRequest(hostPath(workspace), signal), workspace);
}
export async function createHost(workspace: string, definition: HostDefinition): Promise<HostEnrollment> {
  return decodeEnrollment(
    await teamRequest(hostPath(workspace), undefined, decodeHostDefinition(definition), 'POST'),
    workspace,
  );
}
export async function enrollHost(workspace: string, host: Host): Promise<HostEnrollment> {
  decodeHost(host, workspace);
  const response = decodeEnrollment(
    await teamRequest(
      `${hostPath(workspace, host.id)}/enrollment`,
      undefined,
      { expected_revision: host.revision },
      'POST',
    ),
    workspace,
  );
  if (response.host.id !== host.id) throw new Error('Niespójne potwierdzenie wydania biletu.');
  return response;
}
export async function changeHost(
  workspace: string,
  host: Host,
  action: 'approve' | 'drain' | 'revoke',
  drain?: boolean,
): Promise<Host> {
  decodeHost(host, workspace);
  const response = decodeHost(
    await teamRequest(
      `${hostPath(workspace, host.id)}/${action}`,
      undefined,
      { expected_revision: host.revision, ...(action === 'drain' ? { drain } : {}) },
      'POST',
    ),
    workspace,
  );
  if (response.id !== host.id) throw new Error('Niespójne potwierdzenie zmiany hosta.');
  return response;
}
/** GET results from before a mutation cannot replace its newer canonical reply.
 * Invalidations during a mutation are deferred, never interpreted as retries. */
export class HostRegistryReadFence {
  #generation = 0;
  #mutating = false;
  beginRead(): number | null {
    return this.#mutating ? null : ++this.#generation;
  }
  authorityGeneration(): number {
    return this.#generation;
  }
  invalidate(): void {
    this.#generation++;
  }
  beginMutation(): number {
    if (this.#mutating) throw new Error('Zmiana rejestru jest już w toku.');
    this.#mutating = true;
    return ++this.#generation;
  }
  current(generation: number): boolean {
    return generation === this.#generation;
  }
  endMutation(): void {
    this.#mutating = false;
  }
}
/** Authorization is checked by the fresh GET. A GET can preserve an already
 * held pending ticket, but can never recover or mint an enrollment secret. */
export function retainHostEnrollment(
  previous: HostEnrollment | null,
  hosts: Host[],
  now = Date.now(),
): HostEnrollment | null {
  return previous &&
    previous.enrollment.expires_at * 1000 > now &&
    hosts.some(
      (host) =>
        host.id === previous.host.id &&
        host.workspace_id === previous.host.workspace_id &&
        host.status === 'pending' &&
        host.enrollment_epoch === previous.host.enrollment_epoch,
    )
    ? previous
    : null;
}
