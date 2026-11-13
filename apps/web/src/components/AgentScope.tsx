import { useEffect, useState } from 'react';
import type { ReactNode } from 'react';
import { decodeWorkspaces, decodeTeamProjects, teamRequest } from '../lib/teams';
import type { Workspace, TeamProject } from '../lib/teams';
export function AgentScope({
  userId,
  children,
}: {
  userId: string;
  children: (workspace: string, project: string) => ReactNode;
}) {
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]),
    [projects, setProjects] = useState<TeamProject[]>([]),
    [workspace, setWorkspace] = useState(''),
    [project, setProject] = useState(''),
    [error, setError] = useState(''),
    [loading, setLoading] = useState(true);
  useEffect(() => {
    const c = new AbortController();
    setLoading(true);
    setError('');
    teamRequest('/v1/web/workspaces', c.signal)
      .then((v) => {
        if (!c.signal.aborted) setWorkspaces(decodeWorkspaces(v, userId));
      })
      .catch((e) => {
        if (!c.signal.aborted) setError(e.message);
      })
      .finally(() => {
        if (!c.signal.aborted) setLoading(false);
      });
    return () => c.abort();
  }, [userId]);
  useEffect(() => {
    setProjects([]);
    setProject('');
    if (!workspace) return;
    const c = new AbortController();
    setLoading(true);
    setError('');
    teamRequest(`/v1/web/workspaces/${workspace}/projects`, c.signal)
      .then((v) => {
        if (!c.signal.aborted) setProjects(decodeTeamProjects(v, workspace, userId));
      })
      .catch((e) => {
        if (!c.signal.aborted) setError(e.message);
      })
      .finally(() => {
        if (!c.signal.aborted) setLoading(false);
      });
    return () => c.abort();
  }, [workspace, userId]);
  return (
    <div className="agent-workspace">
      <div className="agent-scope">
        <label>
          Zespół
          <select
            data-testid="agent-workspace"
            value={workspace}
            onChange={(e) => {
              setWorkspace(e.target.value);
              setProject('');
            }}
          >
            <option value="">Wybierz zespół</option>
            {workspaces.map((w) => (
              <option key={w.id} value={w.id}>
                {w.name}
              </option>
            ))}
          </select>
        </label>
        <label>
          Projekt zespołu
          <select
            data-testid="agent-project"
            value={project}
            disabled={!workspace || loading}
            onChange={(e) => setProject(e.target.value)}
          >
            <option value="">Wybierz projekt</option>
            {projects.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
          </select>
        </label>
      </div>
      {loading && <p role="status">Wczytywanie zakresu…</p>}
      {error && <p role="alert">{error}</p>}
      {!loading && !workspaces.length && !error && <p>Brak dostępnych zespołów.</p>}
      {workspace && !loading && !projects.length && !error && <p>Brak dostępnych projektów zespołu.</p>}
      {project && children(workspace, project)}
    </div>
  );
}
