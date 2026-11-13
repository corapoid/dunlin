import { remoteBindingsPath, decodeRemoteBinding } from './remote-bindings.ts';
import type { RemoteBinding } from './remote-bindings.ts';
import { teamRequest, TeamRequestError } from './teams.ts';

export type BindingRef = { id: string; revision: number };
export type RemoteBindingAccess = {
  binding: BindingRef;
  workspace_id: string;
  project_id: string;
  host_id: string;
  host_enrollment_epoch: number;
  credential_revision: number;
  export_id: string;
  export_revision: number;
  repo_kind: 'directory' | 'git' | 'linked_worktree';
  checkout_identity_digest: string;
  approved_settings_digest: string;
  principal: { kind: 'human'; id: string };
  grant_revision: number;
  grant_permissions: ('read' | 'execute')[];
  observed_at_ms: number;
  required_worker_permission: 'read';
  observed_state_only: true;
  execution_ready: false;
};
function object(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === 'object' && !Array.isArray(value);
}
export function decodeRemoteBindingAccess(
  value: unknown, expected: RemoteBinding, user: string,
): RemoteBindingAccess {
  if (!object(value) || !object(value.binding)
    || Object.keys(value.binding).some(key => !['id', 'revision'].includes(key))
    || value.binding.id !== expected.id || value.binding.revision !== expected.revision
    || value.required_worker_permission !== 'read' || value.observed_state_only !== true
    || value.execution_ready !== false
    || Object.keys(value).some(key => ![
      'binding', 'workspace_id', 'project_id', 'host_id', 'host_enrollment_epoch',
      'credential_revision', 'export_id', 'export_revision', 'repo_kind',
      'checkout_identity_digest', 'approved_settings_digest', 'principal', 'grant_revision',
      'grant_permissions', 'observed_at_ms', 'required_worker_permission',
      'observed_state_only', 'execution_ready',
    ].includes(key))) {
    throw new Error('Niepoprawna odpowiedź sprawdzenia zdalnego celu.');
  }
  const { binding, required_worker_permission, observed_state_only, execution_ready, ...metadata } = value;
  void binding; void required_worker_permission; void observed_state_only; void execution_ready;
  const checked = decodeRemoteBinding(
    { ...metadata, id: expected.id, revision: expected.revision, active: true },
    expected.workspace_id, expected.project_id,
  );
  const targetFields: (keyof RemoteBinding)[] = [
    'host_id', 'host_enrollment_epoch', 'credential_revision', 'export_id', 'export_revision',
    'repo_kind', 'checkout_identity_digest', 'approved_settings_digest',
  ];
  if (checked.principal.kind !== 'human' || checked.principal.id !== user
    || !checked.grant_permissions.includes('read')
    || targetFields.some(key => checked[key] !== expected[key])) {
    throw new Error('Sprawdzenie nie odpowiada wskazanemu celowi lub bieżącemu użytkownikowi.');
  }
  return structuredClone(value) as RemoteBindingAccess;
}
export type RemoteBindingAccessState = {
  status: 'idle' | 'loading' | 'observed' | 'denied' | 'error';
  result: RemoteBindingAccess | null;
  error: string;
};
const idle = (): RemoteBindingAccessState => ({ status: 'idle', result: null, error: '' });

/** A caller-specific read observation. No saved administrator grant is reused,
 * no binding/session changes, and every check requires an explicit action. */
export class RemoteBindingAccessReader {
  #workspace: string;
  #project: string;
  #user: string;
  #changed: (state: RemoteBindingAccessState) => void;
  #state: RemoteBindingAccessState = idle();
  #generation = 0;
  #stopped = false;
  #abort: AbortController | null = null;
  constructor(
    workspace: string, project: string, user: string,
    changed: (state: RemoteBindingAccessState) => void,
  ) {
    remoteBindingsPath(workspace, project);
    if (!/^[A-Za-z0-9_-]{1,128}$/.test(user)) throw new Error('Niepoprawna tożsamość użytkownika.');
    this.#workspace = workspace;
    this.#project = project;
    this.#user = user;
    this.#changed = changed;
  }
  get state(): RemoteBindingAccessState { return this.#state; }
  #publish(state: RemoteBindingAccessState): void {
    this.#state = state;
    this.#changed(state);
  }
  invalidate(): void {
    this.#generation++;
    this.#abort?.abort();
    this.#abort = null;
    this.#publish(idle());
  }
  stop(): void {
    this.#stopped = true;
    this.#generation++;
    this.#abort?.abort();
    this.#abort = null;
    this.#state = idle();
  }
  async check(binding: RemoteBinding): Promise<void> {
    if (this.#stopped) return;
    const expected = decodeRemoteBinding(binding, this.#workspace, this.#project);
    if (!expected.active) throw new Error('Odwołany binding nie wskazuje aktywnego celu.');
    this.#abort?.abort();
    const controller = new AbortController();
    this.#abort = controller;
    const generation = ++this.#generation;
    const current = () => !this.#stopped && this.#generation === generation;
    this.#publish({ status: 'loading', result: null, error: '' });
    const timer = setTimeout(() => {
      if (!current()) return;
      controller.abort();
      this.#generation++;
      this.#abort = null;
      this.#publish({ status: 'error', result: null,
        error: 'Nie potwierdzono wyniku sprawdzenia w wyznaczonym czasie. Możesz jawnie sprawdzić cel ponownie.' });
    }, 15_000);
    try {
      const value = await teamRequest(
        `${remoteBindingsPath(this.#workspace, this.#project)}/resolve`, controller.signal,
        { binding: { id: expected.id, revision: expected.revision } }, 'POST',
      );
      if (!current() || controller.signal.aborted) return;
      const result = decodeRemoteBindingAccess(value, expected, this.#user);
      this.#publish({ status: 'observed', result, error: '' });
    } catch (error) {
      if (!current()) return;
      const denied = error instanceof TeamRequestError && [401, 403, 404, 409].includes(error.status);
      this.#publish({ status: denied ? 'denied' : 'error', result: null,
        error: denied
          ? 'Nie potwierdzono dostępu do wskazanego celu. Sprawdź rewizję bindingu i własny grant Read na workerze.'
          : 'Nie udało się sprawdzić wskazanego celu. Odczyt nie będzie automatycznie ponawiany.' });
    } finally {
      clearTimeout(timer);
      if (current()) this.#abort = null;
    }
  }
}
