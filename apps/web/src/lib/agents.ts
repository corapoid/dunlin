import { teamRequest } from './teams.ts';
export type AgentMcpTool = { provider_id: string; name: string; definition_digest: string };
export type AgentDefinition = {
  schema_version: 1 | 2;
  name: string;
  description: string;
  harness: string;
  model: string;
  mode: string;
  instructions: string | null;
  skills: { id: string; version: string; instructions: string }[];
  mcp?: AgentMcpTool[];
  max_seconds: number;
  max_output_bytes: number;
};
export type AgentDraft = {
  id: string;
  workspace_id: string;
  project_id: string;
  revision: number;
  definition: AgentDefinition;
  updated_by: string;
  updated_at: number;
};
export type AgentVersion = {
  agent_id: string;
  version: number;
  draft_revision: number;
  definition: AgentDefinition;
  digest: string;
  published_by: string;
  published_at: number;
};
function object(v: unknown): v is Record<string, unknown> {
  return !!v && typeof v === 'object' && !Array.isArray(v);
}
function integer(v: unknown): v is number {
  return Number.isSafeInteger(v) && Number(v) > 0;
}
function id(v: unknown): v is string {
  return typeof v === 'string' && /^[A-Za-z0-9_-]{1,128}$/.test(v);
}
function text(v: unknown, max: number, required = false, multiline = false): v is string {
  return (
    typeof v === 'string' &&
    (!required || !!v.trim()) &&
    new TextEncoder().encode(v).length <= max &&
    ![...v].some((c) => /\p{Cc}/u.test(c) && !(multiline && ['\n', '\r', '\t'].includes(c)))
  );
}
export function agentPath(workspace: string, project: string): string {
  if (!id(workspace) || !id(project)) throw new Error('Niepoprawny zakres agenta.');
  return `/v1/web/workspaces/${workspace}/projects/${project}`;
}
export function decodeDefinition(value: unknown): AgentDefinition {
  if (!object(value)) throw new Error('Niepoprawna definicja agenta.');
  const v: Record<string, unknown> = { description: '', skills: [], instructions: null, ...value };
  if (
    Object.keys(v).some(
      (k) =>
        ![
          'schema_version',
          'name',
          'description',
          'harness',
          'model',
          'mode',
          'instructions',
          'skills',
          'mcp',
          'max_seconds',
          'max_output_bytes',
        ].includes(k),
    ) ||
    ![1,2].includes(Number(v.schema_version)) || typeof v.schema_version !== 'number' ||
    !text(v.name, 128, true) ||
    !text(v.description, 4096, false, true) ||
    !(['opencode', 'codex', 'claude'] as unknown[]).includes(v.harness) ||
    !text(v.model, 256, true) ||
    !text(v.mode, 128, true) ||
    (v.instructions !== null && !text(v.instructions, 16384, false, true)) ||
    !integer(v.max_seconds) ||
    v.max_seconds > 1800 ||
    !integer(v.max_output_bytes) ||
    v.max_output_bytes < 1024 ||
    v.max_output_bytes > 4194304 ||
    !Array.isArray(v.skills) ||
    v.skills.length > 8
  )
    throw new Error('Niepoprawna definicja agenta lub przekroczone limity.');
  if (v.mcp !== undefined) {
    if (!Array.isArray(v.mcp) || v.mcp.length > 32 || v.mcp.length > 0 && v.schema_version !== 2)
      throw new Error('Niepoprawna allowlista MCP.');
    const names=new Set<string>(),providers=new Set<string>();
    for (const tool of v.mcp) {
      const pin=decodeMcpTool(tool),key=JSON.stringify([pin.provider_id,pin.name]);
      if(names.has(key))throw new Error('Powtórzone narzędzie MCP.');
      names.add(key);providers.add(pin.provider_id);
    }
    if(providers.size>8)throw new Error('Dozwolone jest najwyżej 8 dostawców MCP.');
    // Preserve legacy canonical JSON and Rust's omission of empty policies.
    if(!v.mcp.length)delete v.mcp;
  }
  const seen = new Set<string>();
  let bytes = new TextEncoder().encode(v.instructions ?? '').length;
  for (const skill of v.skills) {
    if (
      !object(skill) ||
      Object.keys(skill).some((k) => !['id', 'version', 'instructions'].includes(k)) ||
      !id(skill.id) ||
      typeof skill.version !== 'string' ||
      !/^[A-Za-z0-9_.-]{1,64}$/.test(skill.version) ||
      !text(skill.instructions, 8192, true, true) ||
      seen.has(skill.id)
    )
      throw new Error('Niepoprawne umiejętności agenta.');
    seen.add(skill.id);
    bytes += new TextEncoder().encode(skill.instructions).length;
  }
  if (bytes > 16384 || new TextEncoder().encode(JSON.stringify(v)).length > 65536)
    throw new Error(
      'Instrukcje agenta i umiejętności przekraczają wspólny limit 16 KiB lub limit definicji.',
    );
  return structuredClone(v) as AgentDefinition;
}
export function decodeMcpTool(value: unknown): AgentMcpTool {
  if(!object(value)||Object.keys(value).length!==3
    ||!Object.keys(value).every(k=>['provider_id','name','definition_digest'].includes(k))
    ||typeof value.provider_id!=='string'||!/^[A-Za-z0-9_-]{1,64}$/.test(value.provider_id)
    ||!text(value.name,256,true)||/[*?]/u.test(value.name)
    ||typeof value.definition_digest!=='string'||!/^[a-f0-9]{64}$/.test(value.definition_digest))
    throw new Error('Niepoprawne przypięcie narzędzia MCP.');
  return structuredClone(value) as AgentMcpTool;
}
function timestamp(v: unknown): v is number {
  return Number.isSafeInteger(v) && Number(v) >= 0;
}
export function decodeDraft(v: unknown, workspace: string, project: string): AgentDraft {
  agentPath(workspace, project);
  if (
    !object(v) ||
    !id(v.id) ||
    v.workspace_id !== workspace ||
    v.project_id !== project ||
    !integer(v.revision) ||
    !id(v.updated_by) ||
    !timestamp(v.updated_at)
  )
    throw new Error('Niespójny zakres lub wersja szkicu agenta.');
  return { ...v, definition: decodeDefinition(v.definition) } as AgentDraft;
}
export function decodeVersion(v: unknown): AgentVersion {
  if (
    !object(v) ||
    !id(v.agent_id) ||
    !integer(v.version) ||
    !integer(v.draft_revision) ||
    typeof v.digest !== 'string' ||
    !/^[a-f0-9]{64}$/.test(v.digest) ||
    !id(v.published_by) ||
    !timestamp(v.published_at)
  )
    throw new Error('Niepoprawna opublikowana wersja agenta.');
  return { ...v, definition: decodeDefinition(v.definition) } as AgentVersion;
}
export function decodeDrafts(v: unknown, w: string, p: string): AgentDraft[] {
  if (!object(v) || !Array.isArray(v.drafts) || v.drafts.length > 256)
    throw new Error('Niepoprawna lista agentów.');
  const drafts = v.drafts.map((d) => decodeDraft(d, w, p));
  if (new Set(drafts.map((d) => d.id)).size !== drafts.length) throw new Error('Powtórzony agent.');
  return drafts;
}
export type ProfileCatalog = {
  profiles: AgentVersion[];
  configured_harness: string | null;
  remote_sessions_supported?: boolean;
  mcp_sessions_supported?: boolean;
  remote_mcp_sessions_supported?: boolean;
  can_edit: boolean;
};
export function decodeProfiles(v: unknown): ProfileCatalog {
  if (
    !object(v) ||
    !Array.isArray(v.profiles) ||
    v.profiles.length > 256 ||
    (v.configured_harness !== null &&
      (typeof v.configured_harness !== 'string' ||
        !['opencode', 'codex', 'claude'].includes(v.configured_harness))) ||
    (v.remote_sessions_supported !== undefined && typeof v.remote_sessions_supported !== 'boolean') ||
    (v.mcp_sessions_supported !== undefined && typeof v.mcp_sessions_supported !== 'boolean') ||
    (v.remote_mcp_sessions_supported !== undefined && typeof v.remote_mcp_sessions_supported !== 'boolean') ||
    typeof v.can_edit !== 'boolean'
  )
    throw new Error('Niepoprawny katalog profili.');
  const profiles = v.profiles.map(decodeVersion);
  if (new Set(profiles.map((p) => p.agent_id)).size !== profiles.length)
    throw new Error('Powtórzony profil.');
  return { profiles, configured_harness: v.configured_harness as string | null, can_edit: v.can_edit, ...(v.remote_sessions_supported === undefined ? {} : {remote_sessions_supported: v.remote_sessions_supported}), ...(v.mcp_sessions_supported===undefined?{}:{mcp_sessions_supported:v.mcp_sessions_supported}), ...(v.remote_mcp_sessions_supported===undefined?{}:{remote_mcp_sessions_supported:v.remote_mcp_sessions_supported}) };
}
export async function readProfiles(w: string, p: string, signal?: AbortSignal): Promise<ProfileCatalog> {
  return decodeProfiles(await teamRequest(`${agentPath(w, p)}/agent-profiles`, signal));
}
export async function readDrafts(w: string, p: string, signal?: AbortSignal): Promise<AgentDraft[]> {
  return decodeDrafts(await teamRequest(`${agentPath(w, p)}/agents`, signal), w, p);
}
export async function saveDraft(
  w: string,
  p: string,
  definition: AgentDefinition,
  existing: AgentDraft | null,
): Promise<AgentDraft> {
  const clean = decodeDefinition(definition);
  if (existing) decodeDraft(existing, w, p);
  return decodeDraft(
    await teamRequest(
      `${agentPath(w, p)}/agents${existing ? `/${existing.id}` : ''}`,
      undefined,
      existing ? { expected_revision: existing.revision, definition: clean } : clean,
      existing ? 'PUT' : 'POST',
    ),
    w,
    p,
  );
}
export async function publishDraft(w: string, p: string, draft: AgentDraft): Promise<AgentVersion> {
  decodeDraft(draft, w, p);
  const value = decodeVersion(
    await teamRequest(
      `${agentPath(w, p)}/agents/${draft.id}/publish`,
      undefined,
      { expected_revision: draft.revision },
      'POST',
    ),
  );
  if (value.agent_id !== draft.id || value.draft_revision !== draft.revision)
    throw new Error('Niespójne potwierdzenie publikacji.');
  return value;
}
