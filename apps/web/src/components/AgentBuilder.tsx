import { useEffect, useRef, useState } from 'react';
import { ScopedToolPicker } from './ScopedSecrets';
import { AgentScope } from './AgentScope';
import { readDrafts, readProfiles, saveDraft, publishDraft } from '../lib/agents';
import type { AgentDraft, AgentDefinition, ProfileCatalog } from '../lib/agents';
import { TeamRequestError } from '../lib/teams';
const empty = (harness: string): AgentDefinition => ({
  schema_version: 1,
  name: '',
  description: '',
  harness,
  model: '',
  mode: '',
  instructions: '',
  skills: [],
  max_seconds: 120,
  max_output_bytes: 1048576,
});
function Editor({ workspace, project, userId }: { workspace: string; project: string; userId:string }) {
  const [drafts, setDrafts] = useState<AgentDraft[]>([]),
    [catalog, setCatalog] = useState<ProfileCatalog | null>(null),
    [existing, setExisting] = useState<AgentDraft | null>(null),
    [definition, setDefinition] = useState<AgentDefinition>(empty('')),
    [loading, setLoading] = useState(true),
    [busy, setBusy] = useState(false),
    [error, setError] = useState(''),
    [message, setMessage] = useState(''),
    [stale, setStale] = useState(false);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  useEffect(() => {
    const c = new AbortController();
    Promise.all([readDrafts(workspace, project, c.signal), readProfiles(workspace, project, c.signal)])
      .then(([d, p]) => {
        if (!c.signal.aborted) {
          setDrafts(d);
          setCatalog(p);
          setDefinition(empty(p.configured_harness ?? ''));
        }
      })
      .catch((e) => {
        if (!c.signal.aborted) setError(e.message);
      })
      .finally(() => {
        if (!c.signal.aborted) setLoading(false);
      });
    return () => c.abort();
  }, [workspace, project]);
  function choose(draft: AgentDraft | null) {
    setExisting(draft);
    setDefinition(draft ? structuredClone(draft.definition) : empty(catalog?.configured_harness ?? ''));
    setStale(false);
    setError('');
    setMessage('');
  }
  async function mutate(publish = false) {
    setBusy(true);
    setError('');
    setMessage('');
    try {
      if (publish) {
        if (!existing) throw new Error('Najpierw zapisz szkic.');
        const version = await publishDraft(workspace, project, existing);
        if (alive.current)
          setMessage(
            `Opublikowano wersję ${version.version}. Sesje użyją przypiętej definicji tej wersji.`,
          );
      } else {
        const draft = await saveDraft(workspace, project, definition, existing);
        if (alive.current) {
          setExisting(draft);
          setDefinition(structuredClone(draft.definition));
          setDrafts((all) => [...all.filter((d) => d.id !== draft.id), draft]);
          setMessage('Szkic zapisany. Publikacja wymaga osobnej akcji.');
        }
      }
    } catch (e) {
      if (alive.current) {
        if (e instanceof TeamRequestError && e.status === 409) setStale(true);
        setError(e instanceof Error ? e.message : 'Operacja nie powiodła się.');
      }
    } finally {
      if (alive.current) setBusy(false);
    }
  }
  async function reload() {
    setBusy(true);
    try {
      const all = await readDrafts(workspace, project);
      if (alive.current) {
        setDrafts(all);
        choose(all.find((d) => d.id === existing?.id) ?? null);
      }
    } catch (e) {
      if (alive.current) setError(e instanceof Error ? e.message : 'Nie udało się odświeżyć.');
    } finally {
      if (alive.current) setBusy(false);
    }
  }
  if (loading) return <p role="status">Wczytywanie agentów…</p>;
  const editable = catalog?.can_edit === true;
  const changed = existing ? JSON.stringify(definition) !== JSON.stringify(existing.definition) : true;
  return (
    <section data-testid="agent-builder">
      <h2>Definicje agentów</h2>
      {error && (
        <p role="alert" className="notice error">
          {error}
        </p>
      )}
      {message && <p role="status">{message}</p>}
      {!editable && <p>Tylko odczyt. Edycja i publikacja wymagają zarządzania projektem.</p>}
      <label>
        Agent
        <select
          data-testid="agent-draft"
          value={existing?.id ?? ''}
          disabled={busy}
          onChange={(e) => choose(drafts.find((d) => d.id === e.target.value) ?? null)}
        >
          <option value="">Nowy szkic</option>
          {drafts.map((d) => (
            <option key={d.id} value={d.id}>
              {d.definition.name} · rewizja {d.revision}
            </option>
          ))}
        </select>
      </label>
      {stale && (
        <>
          <p role="alert">
            Szkic zmienił się w innej karcie. Twoje pola zachowano; przed dalszym zapisem wczytaj aktualną
            wersję.
          </p>
          <button disabled={busy} onClick={reload}>
            Wczytaj aktualny szkic
          </button>
        </>
      )}
      <form
        className="agent-form"
        onSubmit={(e) => {
          e.preventDefault();
          void mutate();
        }}
      >
        <fieldset disabled={!editable || busy || stale}>
          <label>
            Nazwa
            <input
              data-testid="agent-name"
              required
              maxLength={128}
              value={definition.name}
              onChange={(e) => setDefinition({ ...definition, name: e.target.value })}
            />
          </label>
          <label>
            Opis
            <textarea
              value={definition.description}
              onChange={(e) => setDefinition({ ...definition, description: e.target.value })}
            />
          </label>
          <label>
            Harness
            {catalog?.remote_sessions_supported?<select data-testid="agent-harness" required value={definition.harness} onChange={e=>setDefinition({...definition,harness:e.target.value})}><option value="">Wybierz harness</option>{['opencode','codex','claude'].map(h=><option key={h} value={h}>{h}</option>)}</select>:<input data-testid="agent-harness" value={definition.harness} readOnly />}
          </label>
          {!catalog?.configured_harness && (
            <p role="alert">
              Serwer nie udostępnia obsługiwanego harnessu. Publikacja lub uruchomienie mogą zostać odrzucone.
            </p>
          )}
          <p className="muted">
            Model i tryb to jawne identyfikatory konfiguracji ACP. Serwer sprawdzi je podczas uruchomienia;
            ten formularz nie deklaruje ich obsługi.
          </p>
          <label>
            Identyfikator modelu
            <input
              data-testid="agent-model"
              required
              value={definition.model}
              onChange={(e) => setDefinition({ ...definition, model: e.target.value })}
            />
          </label>
          <label>
            Identyfikator trybu
            <input
              data-testid="agent-mode"
              required
              value={definition.mode}
              onChange={(e) => setDefinition({ ...definition, mode: e.target.value })}
            />
          </label>
          <label>
            Instrukcje agenta
            <textarea
              data-testid="agent-instructions"
              rows={6}
              value={definition.instructions ?? ''}
              onChange={(e) => setDefinition({ ...definition, instructions: e.target.value || null })}
            />
          </label>
          <h3>Umiejętności przypięte do wersji</h3>
          <p className="muted">
            Do 8 umiejętności; łącznie z instrukcjami agenta do 16 KiB tekstu. Treści stanowią jawny kontekst
            tekstowy ACP; nie są osobną rolą systemową.
          </p>
          {definition.skills.map((skill, index) => (
            <div className="agent-skill" key={index} data-testid="agent-skill">
              <label>
                Identyfikator umiejętności
                <input
                  required
                  value={skill.id}
                  onChange={(e) =>
                    setDefinition({
                      ...definition,
                      skills: definition.skills.map((s, i) =>
                        i === index ? { ...s, id: e.target.value } : s,
                      ),
                    })
                  }
                />
              </label>
              <label>
                Wersja umiejętności
                <input
                  required
                  value={skill.version}
                  onChange={(e) =>
                    setDefinition({
                      ...definition,
                      skills: definition.skills.map((s, i) =>
                        i === index ? { ...s, version: e.target.value } : s,
                      ),
                    })
                  }
                />
              </label>
              <label>
                Instrukcje umiejętności
                <textarea
                  required
                  value={skill.instructions}
                  onChange={(e) =>
                    setDefinition({
                      ...definition,
                      skills: definition.skills.map((s, i) =>
                        i === index ? { ...s, instructions: e.target.value } : s,
                      ),
                    })
                  }
                />
              </label>
              <button
                type="button"
                onClick={() =>
                  setDefinition({ ...definition, skills: definition.skills.filter((_, i) => i !== index) })
                }
              >
                Usuń umiejętność
              </button>
            </div>
          ))}
          <button
            type="button"
            data-testid="agent-skill-add"
            disabled={definition.skills.length >= 8}
            onClick={() =>
              setDefinition({
                ...definition,
                skills: [...definition.skills, { id: '', version: '', instructions: '' }],
              })
            }
          >
            Dodaj umiejętność
          </button>
          <h3>Dozwolone narzędzia MCP</h3>
          <ScopedToolPicker workspace={workspace} project={project} userId={userId} disabled={(definition.mcp?.length??0)>=32} onPin={pin=>setDefinition(current=>({...current,schema_version:2,mcp:[...(current.mcp??[]).filter(p=>p.provider_id!==pin.provider_id||p.name!==pin.name),pin]}))}/>
          <p className="muted">Przypięcie narzędzia znajdziesz w katalogu prywatnego połączenia MCP. Profil przechowuje dostawcę i wersję definicji; połączenie konta wybiera właściciel sesji.</p>
          {!!definition.mcp?.length && <p role="status">Allowlista zostanie zapisana w wersji agenta. Przy tworzeniu sesji wybierzesz prywatne połączenia lub granty. Każde wywołanie wymaga jednorazowej zgody.</p>}
          {(definition.mcp ?? []).map((tool,index)=><div className="agent-skill" key={index} data-testid="agent-mcp-tool">
            <label>Dostawca MCP (ID)<input required maxLength={64} pattern="[A-Za-z0-9_\-]+" value={tool.provider_id} onChange={e=>setDefinition({...definition,mcp:definition.mcp?.map((t,i)=>i===index?{...t,provider_id:e.target.value}:t)})}/></label>
            <label>Nazwa narzędzia<input required maxLength={256} value={tool.name} onChange={e=>setDefinition({...definition,mcp:definition.mcp?.map((t,i)=>i===index?{...t,name:e.target.value}:t)})}/></label>
            <label>Odcisk definicji narzędzia<input required maxLength={64} pattern="[a-f0-9]{64}" value={tool.definition_digest} onChange={e=>setDefinition({...definition,mcp:definition.mcp?.map((t,i)=>i===index?{...t,definition_digest:e.target.value}:t)})}/></label>
            <button type="button" onClick={()=>setDefinition({...definition,mcp:definition.mcp?.filter((_,i)=>i!==index)})}>Usuń narzędzie</button>
          </div>)}
          <button type="button" data-testid="agent-mcp-add" disabled={(definition.mcp?.length??0)>=32} onClick={()=>setDefinition({...definition,schema_version:2,mcp:[...(definition.mcp??[]),{provider_id:'',name:'',definition_digest:''}]})}>Dodaj narzędzie MCP</button>
          <label>
            Limit czasu (sekundy)
            <input
              type="number"
              required
              min={1}
              max={1800}
              value={definition.max_seconds}
              onChange={(e) => setDefinition({ ...definition, max_seconds: Number(e.target.value) })}
            />
          </label>
          <label>
            Limit wyjścia (bajty)
            <input
              type="number"
              required
              min={1024}
              max={4194304}
              value={definition.max_output_bytes}
              onChange={(e) => setDefinition({ ...definition, max_output_bytes: Number(e.target.value) })}
            />
          </label>
          <button className="primary" data-testid="agent-save" disabled={!changed}>
            Zapisz szkic
          </button>
          <button
            type="button"
            data-testid="agent-publish"
            disabled={!existing || changed}
            onClick={() => void mutate(true)}
          >
            Opublikuj zapisaną wersję
          </button>
        </fieldset>
      </form>
    </section>
  );
}
export function AgentBuilder({ userId }: { userId: string }) {
  return (
    <>
      <h1>Agenci</h1>
      <p>Edytuj szkic, a następnie jawnie opublikuj niezmienną wersję do uruchamiania sesji.</p>
      <AgentScope userId={userId}>
        {(w, p) => <Editor userId={userId} key={`${w}:${p}`} workspace={w} project={p} />}
      </AgentScope>
    </>
  );
}
