export type SharingReader = { user_id: string; name: string; active: boolean };
export type SharingView = { session_id: string; owner_id: string; revision: number; readers: SharingReader[]; eligible_readers: { user_id: string; name: string }[]; eligible_readers_truncated?: boolean; audit: { revision: number; actor_id: string; added: string[]; removed: string[]; refreshed: string[]; created_at: string }[] };
function object(value: unknown): value is Record<string, unknown> { return !!value && typeof value === 'object' && !Array.isArray(value); }
function id(value: unknown): value is string { return typeof value === 'string' && value.length > 0 && value.length <= 128 && !/[\u0000-\u001f\u007f]/.test(value); }
function revision(value: unknown): value is number { return Number.isSafeInteger(value) && Number(value) >= 0; }
function ids(value: unknown, limit = 32): value is string[] { return Array.isArray(value) && value.length <= limit && value.every(id) && new Set(value).size === value.length; }
export function sharingPath(session: string): string {
  if (!/^[A-Za-z0-9_-]{1,128}$/.test(session)) throw new Error('Niepoprawny identyfikator sesji.');
  return `/v1/web/interactive-sessions/${session}/sharing`;
}
export function decodeSharing(value: unknown, session: string, owner: string): SharingView {
  sharingPath(session);
  if (!object(value) || value.session_id !== session || value.owner_id !== owner || !id(owner) || !revision(value.revision)
    || !Array.isArray(value.readers) || value.readers.length > 32 || !Array.isArray(value.eligible_readers) || value.eligible_readers.length > 1024
    || (value.eligible_readers_truncated !== undefined && typeof value.eligible_readers_truncated !== 'boolean')
    || !Array.isArray(value.audit) || value.audit.length > 1024) throw new Error('Niespójny zakres udostępniania sesji.');
  const readerIds = new Set<string>(), eligibleIds = new Set<string>(), revisions = new Set<number>();
  function reader(item: unknown, seen: Set<string>): item is { user_id: string; name: string } {
    if (!object(item) || !id(item.user_id) || item.user_id === owner || typeof item.name !== 'string' || item.name.length > 256 || seen.has(item.user_id)) return false;
    seen.add(item.user_id); return true;
  }
  if (value.readers.some((r: unknown) => !reader(r, readerIds) || !('active' in r) || typeof r.active !== 'boolean')
    || value.eligible_readers.some((r: unknown) => !reader(r, eligibleIds))
    || value.audit.some((a: unknown) => {
      if (!object(a) || !revision(a.revision) || a.revision > Number(value.revision) || revisions.has(a.revision) || !id(a.actor_id)
        || !ids(a.added) || !ids(a.removed) || !ids(a.refreshed) || typeof a.created_at !== 'string'
        || !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|\+00:00)$/.test(a.created_at) || !Number.isFinite(Date.parse(a.created_at))) return true;
      revisions.add(a.revision); return false;
    })) throw new Error('Niepoprawna lista odbiorców lub historia udostępniania.');
  return { ...value, eligible_readers_truncated: value.eligible_readers_truncated ?? false } as SharingView;
}
/** Stale grants require an explicit new selection; never refresh them during an unrelated edit. */
export function activeReaderIds(view: SharingView): string[] { return view.readers.filter((reader) => reader.active).map((reader) => reader.user_id); }
export class SharingError extends Error {
  status: number;
  constructor(status: number) { super(status === 401 ? 'Sesja przeglądarki wygasła.' : status === 403 || status === 404 ? 'Zarządzanie udostępnianiem nie jest dostępne.' : 'Nie udało się zapisać udostępnienia.'); this.status = status; }
}
export class SharingConflict extends Error {
  current: SharingView;
  constructor(current: SharingView) { super('Udostępnienie zmieniło się w innej karcie. Wczytano aktualną listę; sprawdź ją przed ponownym zapisem.'); this.current = current; }
}
async function request(session: string, signal?: AbortSignal, body?: { expected_revision: number; reader_ids: string[] }): Promise<Response> {
  return fetch(sharingPath(session), { method: body ? 'PUT' : 'GET', credentials: 'same-origin', cache: 'no-store', redirect: 'error', signal,
    ...(body ? { headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) } : {}) });
}
export async function readSharing(session: string, owner: string, signal?: AbortSignal): Promise<SharingView> {
  const response = await request(session, signal); if (!response.ok) throw new SharingError(response.status);
  return decodeSharing(await response.json(), session, owner);
}
export async function saveSharing(session: string, owner: string, expected: number, readers: string[], signal?: AbortSignal): Promise<SharingView> {
  if (!revision(expected) || !ids(readers) || readers.includes(owner)) throw new Error('Niepoprawna lista odbiorców lub wersja udostępnienia.');
  const response = await request(session, signal, { expected_revision: expected, reader_ids: [...readers] });
  if (response.status === 409) throw new SharingConflict(await readSharing(session, owner, signal));
  if (!response.ok) throw new SharingError(response.status);
  return decodeSharing(await response.json(), session, owner);
}
