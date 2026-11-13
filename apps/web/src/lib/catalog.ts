export type CatalogSnapshot = {
  protocol_version: 1;
  stream_id: string;
  sequence: number;
  workspace_revision?: number;
  server_name: string;
  user: { id: string; name: string; role: string };
  projects: { id: string; name: string }[];
  project_id: string | null;
  sessions: { id: string; title: string; updated_at: number }[];
};

export type CatalogEvent =
  | { type: 'snapshot'; snapshot: CatalogSnapshot }
  | { type: 'resync_required' }
  | { type: 'error'; code: string };

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}
function text(value: unknown, limit = 1024): value is string {
  return typeof value === 'string' && value.length <= limit;
}
function uniqueIds(items: { id: string }[]): boolean {
  return new Set(items.map((item) => item.id)).size === items.length;
}

/** Validate the network boundary, not just TypeScript compile-time assertions. */
export function decodeCatalog(data: string): CatalogEvent {
  if (new TextEncoder().encode(data).length > 2 * 1024 * 1024) throw new Error('Snapshot przekracza limit.');
  const event: unknown = JSON.parse(data);
  if (!record(event)) throw new Error('Niepoprawny komunikat live.');
  if (event.type === 'resync_required') return { type: 'resync_required' };
  if (event.type === 'error' && text(event.code, 80)) return { type: 'error', code: event.code };
  if (event.type !== 'snapshot' || !record(event.snapshot)) throw new Error('Nieobsługiwany protokół live.');
  const s = event.snapshot;
  if (s.protocol_version !== 1 || !text(s.stream_id, 128) || !s.stream_id
    || !Number.isSafeInteger(s.sequence) || Number(s.sequence) < 1 || !text(s.server_name)
    || (s.workspace_revision !== undefined && (!Number.isSafeInteger(s.workspace_revision) || Number(s.workspace_revision) < 0))
    || !record(s.user) || !text(s.user.id, 128) || !text(s.user.name) || !text(s.user.role, 64)
    || (s.project_id !== null && !text(s.project_id, 128))
    || !Array.isArray(s.projects) || s.projects.length > 500
    || !s.projects.every((p) => record(p) && text(p.id, 128) && text(p.name))
    || !Array.isArray(s.sessions) || s.sessions.length > 1000
    || !s.sessions.every((item) => record(item) && text(item.id, 128) && text(item.title)
      && Number.isSafeInteger(item.updated_at) && Number(item.updated_at) >= 0
      && Number(item.updated_at) <= 253_402_300_799)) {
    throw new Error('Niepoprawny snapshot katalogu.');
  }
  const snapshot = s as CatalogSnapshot;
  if (!uniqueIds(snapshot.projects) || !uniqueIds(snapshot.sessions)
    || (snapshot.project_id === null && snapshot.sessions.length !== 0)
    || (snapshot.project_id !== null && !snapshot.projects.some((p) => p.id === snapshot.project_id))) {
    throw new Error('Niespójny zakres snapshotu.');
  }
  return { type: 'snapshot', snapshot };
}

export function acceptSnapshot(previous: CatalogSnapshot | null, next: CatalogSnapshot): boolean {
  return previous === null || previous.stream_id !== next.stream_id || next.sequence > previous.sequence;
}
