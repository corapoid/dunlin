import { useEffect, useRef, useState } from 'react';
import { HostHealthStream } from '../lib/host-health';
import type { HostHealthState } from '../lib/host-health';
const labels = { 1: 'Diagnostyka', 2: 'Metadane eksportów', 3: 'Pojedyncze zadania', 4: 'Sesje trwałe', 5: 'Sesje trwałe' };
export function HostHealthLive({ workspace, host, onForbidden }: {
  workspace: string; host: string; onForbidden: () => void;
}) {
  const [state, setState] = useState<HostHealthState>({ status: 'idle', health: null, error: '' });
  const reader = useRef<HostHealthStream | null>(null);
  const forbidden = useRef(onForbidden);
  forbidden.current = onForbidden;
  useEffect(() => {
    const stream = new HostHealthStream(window.location.origin, workspace, host, (next) => {
      setState(next);
      if (next.status === 'forbidden') forbidden.current();
    });
    reader.current = stream;
    setState(stream.state);
    return () => { stream.stop(); if (reader.current === stream) reader.current = null; };
  }, [workspace, host]);
  const health = state.health;
  const connection = health?.connection;
  const active = state.status === 'live' || state.status === 'connecting';
  return <section className="host-channel" aria-label="Kondycja hosta na żywo" data-testid="host-health-live">
    <h4>Kondycja hosta na żywo</h4>
    <p className="muted">Połączenie i stan wykonania aktualizowane przez WebSocket.</p>
    <button data-testid="host-health-start" disabled={active || state.status === 'forbidden'}
      onClick={() => reader.current?.start()}>
      {state.status === 'connecting' ? 'Łączenie…' : state.status === 'idle' ? 'Obserwuj na żywo' : 'Połącz ponownie'}
    </button>
    {active && <button onClick={() => { reader.current?.stop(); if (reader.current) setState(reader.current.state); }}>Zatrzymaj podgląd</button>}
    {state.error && <p role="alert">{state.error}</p>}
    {health && <div data-testid="host-health-observation">
      <p data-testid="host-health-connection">
        {connection ? connection.heartbeat_sequence === 0 ? 'Połączenie przyjęte · oczekiwanie na heartbeat' : 'Kanał WSS połączony'
          : 'Brak bieżącego potwierdzenia kanału'}
        {health.host.drain && ' · drain aktywny'}
      </p>
      <p className="muted">Obserwacja: <time dateTime={new Date(health.observed_at_ms).toISOString()}>{new Date(health.observed_at_ms).toLocaleTimeString('pl')}</time></p>
      {connection && <dl>
        <dt>Transport</dt><dd data-testid="host-health-protocol">v{connection.protocol_version} · {labels[connection.protocol_version]}</dd>
        <dt>Ostatni heartbeat</dt><dd data-testid="host-health-heartbeat">{connection.heartbeat_sequence}</dd>
      </dl>}
      <dl>
        <dt>Rezerwacje sesji</dt><dd>{health.runtime.session_reservations}</dd>
        <dt>Aktywne sesje</dt><dd>{health.runtime.active_sessions}</dd>
        <dt>Aktywne tury</dt><dd>{health.runtime.active_turns}</dd>
        <dt>Rezerwacje zadań</dt><dd>{health.runtime.one_shot_reservations}</dd>
        <dt>Niepewne rezerwacje</dt><dd>{health.runtime.uncertain_reservations}</dd>
        <dt>ACK do potwierdzenia</dt><dd>{health.runtime.unconfirmed_acknowledgements}</dd>
      </dl>
      {health.runtime.uncertain_reservations > 0 && <p className="notice">Niepewne rezerwacje czekają na potwierdzone zakończenie pracy workera.</p>}
      <p>Raportowana wersja: {health.host.capabilities?.version ?? 'brak raportu'} · harnessy: {health.host.capabilities?.harnesses.join(', ') || 'brak raportu'}</p>
    </div>}
  </section>;
}
