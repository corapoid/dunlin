import { hostPath, HostRegistryReadFence } from './hosts.ts';
import { teamRequest, TeamRequestError } from './teams.ts';
export type RemoteBinding = {
  id: string;
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
  principal: { kind: 'human' | 'service'; id: string };
  grant_revision: number;
  grant_permissions: ('read' | 'execute')[];
  revision: number;
  active: boolean;
  observed_at_ms: number;
};
export type RemoteBindingInput = { host_id: string; export_id: string; expected_export_revision: number };
function object(v: unknown): v is Record<string, unknown> {
  return !!v && typeof v === 'object' && !Array.isArray(v);
}
function id(v: unknown): v is string {
  return typeof v === 'string' && /^[A-Za-z0-9_-]{1,128}$/.test(v);
}
function integer(v: unknown, min = 1): v is number {
  return Number.isSafeInteger(v) && Number(v) >= min;
}
function digest(v: unknown): v is string {
  return typeof v === 'string' && /^[a-f0-9]{64}$/.test(v);
}
export function remoteBindingsPath(workspace: string, project: string, binding?: string): string {
  hostPath(workspace);
  if (!id(project) || (binding !== undefined && !id(binding)))
    throw new Error('Niepoprawny zakres zdalnego bindingu.');
  return `/v1/web/workspaces/${workspace}/projects/${project}/remote-bindings${binding ? `/${binding}` : ''}`;
}
export function decodeRemoteBinding(v: unknown, workspace: string, project: string): RemoteBinding {
  remoteBindingsPath(workspace, project);
  if (
    !object(v) ||
    Object.keys(v).some(
      (k) =>
        ![
          'id',
          'workspace_id',
          'project_id',
          'host_id',
          'host_enrollment_epoch',
          'credential_revision',
          'export_id',
          'export_revision',
          'repo_kind',
          'checkout_identity_digest',
          'approved_settings_digest',
          'principal',
          'grant_revision',
          'grant_permissions',
          'revision',
          'active',
          'observed_at_ms',
        ].includes(k),
    ) ||
    !id(v.id) ||
    v.workspace_id !== workspace ||
    v.project_id !== project ||
    !id(v.host_id) ||
    !integer(v.host_enrollment_epoch) ||
    !integer(v.credential_revision) ||
    !id(v.export_id) ||
    !integer(v.export_revision) ||
    typeof v.repo_kind !== 'string' ||
    !['directory', 'git', 'linked_worktree'].includes(v.repo_kind) ||
    !digest(v.checkout_identity_digest) ||
    !digest(v.approved_settings_digest) ||
    !object(v.principal) ||
    Object.keys(v.principal).some((k) => !['kind', 'id'].includes(k)) ||
    !['human', 'service'].includes(String(v.principal.kind)) ||
    typeof v.principal.kind !== 'string' ||
    !id(v.principal.id) ||
    !integer(v.grant_revision) ||
    !Array.isArray(v.grant_permissions) ||
    v.grant_permissions.length > 2 ||
    v.grant_permissions.some((p) => !['read', 'execute'].includes(p)) ||
    new Set(v.grant_permissions).size !== v.grant_permissions.length ||
    !integer(v.revision) ||
    typeof v.active !== 'boolean' ||
    !integer(v.observed_at_ms, 0)
  )
    throw new Error('Niepoprawne lub niespójne metadane zdalnego bindingu.');
  return structuredClone(v) as RemoteBinding;
}
export function decodeRemoteBindings(v: unknown, workspace: string, project: string): RemoteBinding[] {
  if (
    !object(v) ||
    Object.keys(v).some((k) => !['bindings', 'observed_state_only', 'execution_ready'].includes(k)) ||
    v.observed_state_only !== true ||
    v.execution_ready !== false ||
    !Array.isArray(v.bindings) ||
    v.bindings.length > 16
  )
    throw new Error('Niepoprawna lista zaobserwowanych zdalnych bindingów.');
  const bindings = v.bindings.map((b) => decodeRemoteBinding(b, workspace, project));
  if (
    new Set(bindings.map((b) => b.id)).size !== bindings.length ||
    new Set(bindings.map((b) => `${b.host_id}:${b.export_id}`)).size !== bindings.length
  )
    throw new Error('Powtórzony zdalny binding.');
  return bindings;
}
export function decodeRemoteBindingInput(v: unknown): RemoteBindingInput {
  if (
    !object(v) ||
    Object.keys(v).some((k) => !['host_id', 'export_id', 'expected_export_revision'].includes(k)) ||
    !id(v.host_id) ||
    !id(v.export_id) ||
    !integer(v.expected_export_revision)
  )
    throw new Error('Wymagany jest host, nieprzezroczysty identyfikator exportu i jego jawna rewizja.');
  return { host_id: v.host_id, export_id: v.export_id, expected_export_revision: v.expected_export_revision };
}
export async function readRemoteBindings(
  workspace: string,
  project: string,
  signal?: AbortSignal,
): Promise<RemoteBinding[]> {
  return decodeRemoteBindings(
    await teamRequest(remoteBindingsPath(workspace, project), signal),
    workspace,
    project,
  );
}
export async function saveRemoteBinding(
  workspace: string,
  project: string,
  input: RemoteBindingInput,
  existing: RemoteBinding | null,
): Promise<RemoteBinding> {
  const body = decodeRemoteBindingInput(input);
  if (existing) decodeRemoteBinding(existing, workspace, project);
  const result = decodeRemoteBinding(
    await teamRequest(
      remoteBindingsPath(workspace, project, existing?.id),
      undefined,
      existing ? { ...body, expected_revision: existing.revision } : body,
      existing ? 'PUT' : 'POST',
    ),
    workspace,
    project,
  );
  if (
    (existing && result.id !== existing.id) ||
    result.host_id !== body.host_id ||
    result.export_id !== body.export_id ||
    result.export_revision !== body.expected_export_revision
  )
    throw new Error('Niespójne potwierdzenie zdalnego bindingu.');
  return result;
}
export async function revokeRemoteBinding(
  workspace: string,
  project: string,
  existing: RemoteBinding,
): Promise<RemoteBinding> {
  decodeRemoteBinding(existing, workspace, project);
  const result = decodeRemoteBinding(
    await teamRequest(
      `${remoteBindingsPath(workspace, project, existing.id)}/revoke`,
      undefined,
      { expected_revision: existing.revision },
      'POST',
    ),
    workspace,
    project,
  );
  if (result.id !== existing.id || result.active)
    throw new Error('Niespójne potwierdzenie odwołania bindingu.');
  return result;
}
export type RemoteBindingsState = {
  status: 'loading' | 'ready' | 'mutating' | 'error' | 'forbidden';
  bindings: RemoteBinding[];
  error: string;
  needsReload: boolean;
};
/** All writes are explicit and single-attempt. Read generations fence metadata
 * replies; a forbidden invalidation also fences a pending mutation's result. */
export class RemoteBindingsController {
  #workspace: string;
  #project: string;
  #changed: (state: RemoteBindingsState) => void;
  #state: RemoteBindingsState = { status: 'loading', bindings: [], error: '', needsReload: false };
  #fence = new HostRegistryReadFence();
  #stopped = false;
  #deferred = false;
  #read: AbortController | null = null;
  constructor(workspace: string, project: string, changed: (state: RemoteBindingsState) => void) {
    remoteBindingsPath(workspace, project);
    this.#workspace = workspace;
    this.#project = project;
    this.#changed = changed;
  }
  get state(): RemoteBindingsState {
    return this.#state;
  }
  #publish(state: RemoteBindingsState) {
    if (this.#stopped) return;
    this.#state = state;
    this.#changed(state);
  }
  deny() {
    this.#fence.invalidate();
    this.#read?.abort();
    this.#publish({
      status: 'forbidden',
      bindings: [],
      error: 'Brak dostępu. Zarządzanie zdalnym bindingiem wymaga aktualnych uprawnień do hostów i projektu.',
      needsReload: true,
    });
  }
  stop() {
    this.#stopped = true;
    this.#fence.invalidate();
    this.#read?.abort();
    this.#state = { status: 'loading', bindings: [], error: '', needsReload: false };
  }
  async refresh(explicit = false): Promise<void> {
    if (this.#stopped) return;
    this.#read?.abort();
    const controller = new AbortController();
    this.#read = controller;
    const read = this.#fence.beginRead(),
      authorityOnly = read === null,
      generation = read ?? this.#fence.authorityGeneration();
    if (authorityOnly) this.#deferred = true;
    else this.#publish({ ...this.#state, status: 'loading', bindings: [] });
    const current = () => !this.#stopped && !controller.signal.aborted && this.#fence.current(generation);
    try {
      const bindings = await readRemoteBindings(this.#workspace, this.#project, controller.signal);
      if (current() && !authorityOnly)
        this.#publish({
          status: 'ready',
          bindings,
          error: explicit ? '' : this.#state.error,
          needsReload: explicit ? false : this.#state.needsReload,
        });
    } catch (error) {
      if (!current()) return;
      const forbidden = error instanceof TeamRequestError && [401, 403, 404].includes(error.status);
      if (forbidden) this.deny();
      else {
        if (authorityOnly) this.#fence.invalidate();
        this.#publish({
          status: 'error',
          bindings: [],
          error: 'Nie udało się odczytać zdalnych bindingów. Odśwież dane jawnie przed kolejną zmianą.',
          needsReload: true,
        });
      }
    } finally {
      if (this.#read === controller) this.#read = null;
    }
  }
  async #mutate(operation: () => Promise<RemoteBinding>): Promise<RemoteBinding | null> {
    if (this.#stopped || this.#state.status !== 'ready' || this.#state.needsReload)
      throw new Error('Wczytaj aktualne bindingi przed zmianą.');
    const generation = this.#fence.beginMutation();
    this.#read?.abort();
    this.#publish({ ...this.#state, status: 'mutating', error: '' });
    try {
      const binding = await operation();
      if (this.#stopped || !this.#fence.current(generation)) return null;
      this.#publish({
        status: 'ready',
        bindings: [...this.#state.bindings.filter((b) => b.id !== binding.id), binding],
        error: '',
        needsReload: false,
      });
      return binding;
    } catch (error) {
      if (!this.#stopped && this.#fence.current(generation)) {
        if (error instanceof TeamRequestError && [401, 403, 404].includes(error.status)) this.deny();
        else
          this.#publish({
            ...this.#state,
            status: 'error',
            error:
              error instanceof TeamRequestError && error.status === 409
                ? 'Binding lub export zmienił się. Wczytaj zapisany stan i sprawdź cel oraz rewizje przed kolejną zmianą.'
                : 'Nie potwierdzono wyniku zmiany. Nie ponawiamy jej automatycznie; odczytaj zapisany stan przed nową decyzją.',
            needsReload: true,
          });
      }
      return null;
    } finally {
      this.#fence.endMutation();
      if (!this.#stopped && this.#deferred) {
        this.#deferred = false;
        void this.refresh();
      }
    }
  }
  async save(
    input: RemoteBindingInput,
    selected: string | null,
    expectedRevision?: number,
  ): Promise<RemoteBinding | null> {
    const existing = selected ? this.#state.bindings.find((b) => b.id === selected) : null;
    if (selected && !existing) throw new Error('Wybrany binding nie znajduje się w aktualnym odczycie.');
    if (existing && existing.revision !== expectedRevision)
      throw new Error('Zapis zmienił się od wyboru bindingu. Wczytaj jego aktualną wersję do formularza.');
    const command = decodeRemoteBindingInput(input);
    return this.#mutate(() => saveRemoteBinding(this.#workspace, this.#project, command, existing ?? null));
  }
  async revoke(selected: string, expectedRevision: number): Promise<RemoteBinding | null> {
    const binding = this.#state.bindings.find((b) => b.id === selected);
    if (!binding) throw new Error('Wczytaj wybrany binding przed odwołaniem.');
    if (binding.revision !== expectedRevision)
      throw new Error('Binding zmienił się od wyboru. Sprawdź aktualną rewizję przed odwołaniem.');
    return this.#mutate(() => revokeRemoteBinding(this.#workspace, this.#project, binding));
  }
}
