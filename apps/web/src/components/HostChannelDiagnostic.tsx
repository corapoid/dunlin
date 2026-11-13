import { useEffect, useRef, useState } from 'react';
import { HostChannelReader, channelWarnings } from '../lib/host-channel';
import type { HostChannelState } from '../lib/host-channel';
export function HostChannelDiagnostic({
  workspace,
  host,
  onForbidden,
}: {
  workspace: string;
  host: string;
  onForbidden: () => void;
}) {
  const [state, setState] = useState<HostChannelState>({
    status: 'idle',
    diagnostic: null,
    readAt: null,
    error: '',
  });
  const reader = useRef<HostChannelReader | null>(null);
  const forbidden = useRef(onForbidden);
  forbidden.current = onForbidden;
  useEffect(() => {
    const client = new HostChannelReader(workspace, host, setState, () => forbidden.current());
    reader.current = client;
    setState(client.state);
    return () => {
      client.stop();
      if (reader.current === client) reader.current = null;
    };
  }, [workspace, host]);
  const diagnostic = state.diagnostic;
  const channel = diagnostic?.channel;
  const warnings = diagnostic && state.readAt !== null ? channelWarnings(diagnostic, state.readAt) : null;
  return (
    <section
      className="host-channel"
      data-testid="host-channel-diagnostic"
      aria-label="Diagnostyka zapisanego kanału"
    >
      <h4>Kanał sterujący · diagnostyka zapisu</h4>
      <p className="muted">
        Jawny odczyt zapisanego stanu. Rekord nie potwierdza bieżącego połączenia, zdrowia workera ani
        uprawnień do wykonywania zadań.
      </p>
      <button
        data-testid="host-channel-read"
        disabled={state.status === 'loading' || state.status === 'forbidden'}
        onClick={() => void reader.current?.read()}
      >
        {state.status === 'loading' ? 'Odczytywanie diagnostyki…' : 'Odczytaj diagnostykę kanału'}
      </button>
      {state.error && <p role="alert">{state.error}</p>}
      {diagnostic && state.readAt !== null && (
        <div data-testid="host-channel-record">
          <p>
            Odczytano:{' '}
            <time dateTime={new Date(state.readAt).toISOString()}>
              {new Date(state.readAt).toLocaleString('pl')}
            </time>
            . Dane mogą już być nieaktualne.
          </p>
          {!channel ? (
            <p>Brak zapisanego kanału. Nie jest to test łączności hosta.</p>
          ) : (
            <>
              <p data-testid="host-channel-phase">
                Zapisana faza: <code>{channel.phase}</code>
              </p>
              {channel.phase === 'active' && (
                <p>
                  Faza active pochodzi z trwałego rekordu; nie oznacza potwierdzonej aktywności w tej chwili.
                </p>
              )}
              {warnings?.differentBoot && (
                <p className="notice" data-testid="host-channel-old-boot">
                  Rekord pochodzi z innego uruchomienia kontrolera. Nie potwierdza bieżącego kanału.
                </p>
              )}
              {warnings?.expiredAtRead && (
                <p className="notice" data-testid="host-channel-expired">
                  Według zegara przeglądarki zapisany termin lease minął już przy odczycie.
                </p>
              )}
              <dl>
                <dt>Identyfikator kanału</dt>
                <dd>
                  <code>{channel.lease.channel_id}</code>
                </dd>
                <dt>Epoch lease</dt>
                <dd>{channel.lease.epoch}</dd>
                <dt>Ostatnia zapisana sekwencja</dt>
                <dd>{channel.lease.last_sequence}</dd>
                <dt>Zapisany termin lease</dt>
                <dd>
                  <time dateTime={new Date(channel.lease.expires_at_ms).toISOString()}>
                    {new Date(channel.lease.expires_at_ms).toLocaleString('pl')}
                  </time>
                </dd>
                <dt>Boot ID kontrolera w rekordzie</dt>
                <dd>
                  <code>{channel.lease.controller_boot_id}</code>
                </dd>
                <dt>Bieżący boot ID kontrolera z odpowiedzi</dt>
                <dd>
                  <code>{diagnostic.current_controller_boot_id}</code>
                </dd>
                <dt>Boot ID workera w rekordzie</dt>
                <dd>
                  <code>{channel.lease.worker_boot_id}</code>
                </dd>
              </dl>
            </>
          )}
          <p>Zapis diagnostyczny kanału. Bieżącą obserwację pokazuje podgląd na żywo.</p>
        </div>
      )}
    </section>
  );
}
