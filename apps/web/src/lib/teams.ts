export const workspaceRoles = ['owner', 'admin', 'member', 'reviewer', 'viewer'] as const;
export type WorkspaceRole = typeof workspaceRoles[number];
export type Membership = {
  workspace_id: string; user_id: string; role: WorkspaceRole; active: boolean; revision: number;
};
export type Workspace = { id: string; name: string; created_at: number; membership: Membership };
export type TeamProject = {
  id: string; workspace_id: string; name: string; created_at: number;
  authority: { workspace_id: string; project_id: string; user_id: string; membership_revision: number; grant_revision: number };
};

function object(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}
function id(value: unknown): value is string {
  return typeof value === 'string' && value.length > 0 && value.length <= 128;
}
function revision(value: unknown): value is number {
  return Number.isSafeInteger(value) && Number(value) > 0;
}

export function decodeMembership(value: unknown): Membership {
  if (!object(value) || !id(value.workspace_id) || !id(value.user_id)
    || !workspaceRoles.includes(value.role as WorkspaceRole) || typeof value.active !== 'boolean' || !revision(value.revision)) {
    throw new Error('Niepoprawna odpowiedź dotycząca członkostwa.');
  }
  return value as Membership;
}

export function decodeWorkspaces(value: unknown, userId: string): Workspace[] {
  if (!Array.isArray(value) || value.length > 128) throw new Error('Niepoprawny katalog zespołów.');
  const ids = new Set<string>();
  return value.map((item: unknown) => {
    if (!object(item) || !id(item.id) || typeof item.name !== 'string' || item.name.length > 120
      || !Number.isSafeInteger(item.created_at)) throw new Error('Niepoprawny workspace.');
    const membership = decodeMembership(item.membership);
    if (!membership.active || membership.workspace_id !== item.id || membership.user_id !== userId || ids.has(item.id)) {
      throw new Error('Niespójny zakres katalogu zespołów.');
    }
    ids.add(item.id);
    return { id: item.id, name: item.name, created_at: Number(item.created_at), membership };
  });
}

export function decodeMembers(value: unknown, workspace: string): Membership[] {
  if (!Array.isArray(value) || value.length > 256) throw new Error('Niepoprawna lista członków.');
  const ids = new Set<string>();
  return value.map((item: unknown) => {
    const member = decodeMembership(item);
    if (member.workspace_id !== workspace || ids.has(member.user_id)) throw new Error('Niespójny zakres członkostwa.');
    ids.add(member.user_id);
    return member;
  });
}

export function decodeTeamProjects(value: unknown, workspace: string, user: string): TeamProject[] {
  if (!Array.isArray(value) || value.length > 256) throw new Error('Niepoprawny katalog projektów zespołu.');
  const ids = new Set<string>();
  return value.map((item: unknown) => {
    if (!object(item) || !id(item.id) || item.workspace_id !== workspace || typeof item.name !== 'string'
      || !item.name.trim() || item.name.length > 120 || !Number.isSafeInteger(item.created_at)
      || !object(item.authority) || item.authority.workspace_id !== workspace || item.authority.project_id !== item.id
      || item.authority.user_id !== user || !revision(item.authority.membership_revision)
      || !revision(item.authority.grant_revision) || ids.has(item.id)) throw new Error('Niespójny zakres projektu zespołowego.');
    ids.add(item.id);
    return item as TeamProject;
  });
}

export class TeamRequestError extends Error {
  readonly status: number;
  constructor(status: number) {
    super(status === 401 ? 'Sesja wygasła.' : status === 403 ? 'Nie masz uprawnień do tej operacji.'
      : status === 409 ? 'Konflikt stanu. Odśwież dane; zespół musi zachować aktywnego właściciela.'
      : 'Nie udało się wykonać operacji zespołowej.');
    this.status = status;
  }
}

export async function teamRequest(path: string, signal?: AbortSignal, body?: unknown, method = 'GET'): Promise<unknown> {
  // Callers construct fixed same-origin paths; never accept external endpoints.
  if (path !== '/v1/web/invitations/accept' && !/^\/v1\/web\/workspaces(?:\/[A-Za-z0-9_-]+)*$/.test(path)) throw new Error('Niepoprawna ścieżka operacji.');
  const response = await fetch(path, { method, credentials: 'same-origin', cache: 'no-store', redirect: 'error', signal,
    ...(body === undefined ? {} : { headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) }),
  });
  if (!response.ok) throw new TeamRequestError(response.status);
  return response.json();
}
