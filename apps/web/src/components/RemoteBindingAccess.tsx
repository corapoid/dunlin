import { useEffect, useRef, useState } from 'react';
import { RemoteBindingAccessReader } from '../lib/remote-binding-access';
import type { RemoteBindingAccessState } from '../lib/remote-binding-access';
import type { RemoteBinding } from '../lib/remote-bindings';

function observedTime(value: number): string {
  const date = new Date(value);
  return Number.isFinite(date.getTime()) ? date.toLocaleString('pl') : 'Czas obserwacji niedostępny';
}

export function RemoteBindingAccess({ binding, userId, disabled }: {
  binding: RemoteBinding; userId: string; disabled: boolean;
}) {
  const [state, setState] = useState<RemoteBindingAccessState>({ status: 'idle', result: null, error: '' });
  const client = useRef<RemoteBindingAccessReader | null>(null);
  useEffect(() => {
    const reader = new RemoteBindingAccessReader(binding.workspace_id, binding.project_id, userId, setState);
    client.current = reader;
    setState(reader.state);
    return () => { reader.stop(); if (client.current === reader) client.current = null; };
  }, [binding.workspace_id, binding.project_id, userId]);
  return (
    <section data-testid="remote-binding-access">
      <h4>Dostęp bieżącego użytkownika do wskazanego celu</h4>
      <p>Sprawdzenie odczytuje bieżący descriptor exportu oraz własny grant Read użytkownika.
        Zapisany grant administratora nie zastępuje Twojego grantu na workerze.</p>
      <button data-testid="remote-binding-resolve"
        disabled={disabled || !binding.active || state.status === 'loading'}
        onClick={() => void client.current?.check(binding)}>
        {state.status === 'loading' ? 'Sprawdzanie dostępu…' : 'Sprawdź dostęp do wskazanego celu'}
      </button>
      {state.error && <p role="alert" data-testid="remote-binding-access-error">{state.error}</p>}
      {state.result && (
        <div data-testid="remote-binding-access-observed">
          <p>Zaobserwowano dostęp Read dla użytkownika <code>{state.result.principal.id}</code>
            {' '}· rewizja grantu {state.result.grant_revision}.</p>
          <p>Binding {state.result.binding.id} · rewizja {state.result.binding.revision}
            {' '}· export {state.result.export_id} · rewizja {state.result.export_revision}.</p>
          <p>Ostatnia obserwacja: {observedTime(state.result.observed_at_ms)}.</p>
          <p>To odczyt chwili sprawdzenia. Nie rezerwuje celu, nie potwierdza dostępu Execute
            ani gotowości do zdalnego wykonania i nie uruchamia sesji.</p>
        </div>
      )}
    </section>
  );
}
