import { useEffect, useRef, useState } from 'react';

/** The server owns state, nonce, PKCE and provider tokens. */
export function OidcLogin() {
  const [enabled, setEnabled] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const pending = useRef<AbortController | null>(null);
  useEffect(() => {
    const controller = new AbortController();
    fetch('/v1/web/oidc/config', { credentials: 'same-origin', cache: 'no-store', redirect: 'error', signal: controller.signal })
      .then(async (response) => { if (response.ok) { const value: unknown = await response.json(); if (!controller.signal.aborted) setEnabled(typeof value === 'object' && value !== null && 'enabled' in value && value.enabled === true); } })
      .catch(() => {});
    return () => { controller.abort(); pending.current?.abort(); };
  }, []);

  async function login() {
    const controller = new AbortController();
    pending.current?.abort(); pending.current = controller;
    setBusy(true); setError('');
    try {
      const response = await fetch('/v1/web/oidc/start', { method: 'POST', credentials: 'same-origin', cache: 'no-store', redirect: 'error', signal: controller.signal });
      if (!response.ok) throw new Error('Logowanie kontem organizacji jest chwilowo niedostępne.');
      const value: unknown = await response.json();
      if (controller.signal.aborted) return;
      if (!value || typeof value !== 'object' || !('authorization_url' in value) || typeof value.authorization_url !== 'string') throw new Error('Niepoprawna odpowiedź logowania.');
      const url = new URL(value.authorization_url);
      if (url.username || url.password || url.hash || !(url.protocol === 'https:' || (url.protocol === 'http:' && ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname)))) throw new Error('Niepoprawny adres logowania.');
      window.location.assign(url.href);
    } catch (cause) {
      if (!controller.signal.aborted) setError(cause instanceof Error ? cause.message : 'Nie udało się rozpocząć logowania.');
    } finally { if (!controller.signal.aborted) setBusy(false); }
  }
  if (!enabled) return null;
  return <div className="pairing-form"><button type="button" onClick={login} disabled={busy}>{busy ? 'Przekierowanie…' : 'Zaloguj kontem organizacji'}</button>{error && <p role="alert" className="error">{error}</p>}</div>;
}
