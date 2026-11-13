import { useEffect, useState } from 'react';
import type { FormEvent } from 'react';
import { Frame, FrameDescription, FrameHeader, FramePanel, FrameTitle } from './reui/frame';
import { decodeTeamProjects, teamRequest, TeamRequestError } from '../lib/teams';
import type { TeamProject, Workspace } from '../lib/teams';
import { RemoteBindings } from './RemoteBindings';
import { LocalBinding } from './LocalBinding';

type Props = { workspace: Workspace; userId: string; revisionKey: string; onExpired: () => void };

export function TeamProjects({ workspace, userId, revisionKey, onExpired }: Props) {
  const [projects, setProjects] = useState<TeamProject[] | null>(null);
  const [error, setError] = useState('');
  const [refresh, setRefresh] = useState(0);
  const [busy, setBusy] = useState(false);
  const [bindingProject, setBindingProject] = useState<string | null>(null);
  const [localProject, setLocalProject] = useState<string | null>(null);
  const canCreate = ['owner', 'admin'].includes(workspace.membership.role);
  const path = `/v1/web/workspaces/${encodeURIComponent(workspace.id)}/projects`;

  useEffect(() => {
    const controller = new AbortController();
    setProjects(null);
    setError('');
    teamRequest(path, controller.signal)
      .then((value) => {
        if (!controller.signal.aborted) setProjects(decodeTeamProjects(value, workspace.id, userId));
      })
      .catch((cause: unknown) => {
        if (controller.signal.aborted) return;
        if (cause instanceof TeamRequestError && cause.status === 401) onExpired();
        setError(cause instanceof Error ? cause.message : 'Nie udało się odczytać projektów.');
      });
    return () => controller.abort();
  }, [path, workspace.id, userId, revisionKey, refresh, onExpired]);

  async function create(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = event.currentTarget;
    setBusy(true);
    setError('');
    try {
      await teamRequest(
        path,
        undefined,
        { name: String(new FormData(form).get('name') ?? '').trim() },
        'POST',
      );
      form.reset();
      setRefresh((value) => value + 1);
    } catch (cause) {
      if (cause instanceof TeamRequestError && cause.status === 401) onExpired();
      setError(cause instanceof Error ? cause.message : 'Nie udało się utworzyć projektu.');
    } finally {
      setBusy(false);
    }
  }

  return (
    <Frame>
      <FrameHeader>
        <FrameTitle>Projekty zespołu</FrameTitle>
        <FrameDescription>
          Widoczne wyłącznie przez aktualny grant i rolę. Projekt logiczny nie daje dostępu do checkoutu hosta
          ani prywatnych sesji.
        </FrameDescription>
      </FrameHeader>
      <FramePanel>
        {error && (
          <p role="alert" className="error">
            {error}
          </p>
        )}
        {projects === null ? (
          !error && <p role="status">Ładowanie projektów…</p>
        ) : projects.length === 0 ? (
          <p className="muted">Brak jawnie udostępnionych projektów zespołu.</p>
        ) : (
          <ul className="team-members">
            {projects.map((project) => (
              <li key={project.id}>
                <strong>{project.name}</strong>
                <p className="muted">
                  {project.id} · Grant {project.authority.grant_revision}
                </p>
                <button
                  data-testid="project-remote-bindings"
                  data-project-id={project.id}
                  aria-pressed={bindingProject === project.id}
                  onClick={() => setBindingProject((current) => (current === project.id ? null : project.id))}
                >
                  Zdalne bindingi
                </button>
                <button data-testid="project-local-binding" data-project-id={project.id}
                  aria-pressed={localProject === project.id}
                  onClick={() => setLocalProject(current => current === project.id ? null : project.id)}>
                  Lokalny checkout
                </button>
              </li>
            ))}
          </ul>
        )}
        {bindingProject &&
          (projects === null || projects.some((project) => project.id === bindingProject)) && (
            <RemoteBindings
              key={`${userId}:${workspace.id}:${bindingProject}`}
              workspace={workspace.id}
              project={bindingProject}
              revisionKey={revisionKey}
              userId={userId}
            />
          )}
        {localProject && projects?.some(project => project.id === localProject) && (
          <LocalBinding key={`${userId}:${workspace.id}:${localProject}:${revisionKey}`}
            workspace={workspace.id} project={localProject}/>
        )}
        {canCreate && (
          <form onSubmit={create} className="team-inline-form">
            <label htmlFor="team-project-name">Nowy projekt logiczny</label>
            <input id="team-project-name" name="name" required maxLength={120} />
            <button disabled={busy}>Utwórz projekt</button>
          </form>
        )}
      </FramePanel>
    </Frame>
  );
}
