/** Infrastructure endpoints only. External provider APIs use separate policies. */
export function validateEndpoint(value: string): URL {
  let endpoint: URL;
  try { endpoint = new URL(value); } catch { throw new Error('Niepoprawny adres serwera.'); }
  const host = endpoint.hostname.toLowerCase();
  const local = host === 'localhost' || host === '[::1]' || /^127(?:\.\d{1,3}){3}$/.test(host);
  const labels = host.split('.');
  const tailscale = labels.length >= 3 && host.endsWith('.ts.net') && labels.every(
    (label) => /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/.test(label),
  );
  const supported = ['http:', 'https:', 'ws:', 'wss:'].includes(endpoint.protocol);
  const secure = endpoint.protocol === 'https:' || endpoint.protocol === 'wss:';
  if (!supported || (!local && (!tailscale || !secure)) || endpoint.username || endpoint.password
    || endpoint.search || endpoint.hash || endpoint.pathname !== '/') {
    throw new Error('Użyj prywatnego adresu Tailscale HTTPS albo loopback na tym komputerze.');
  }
  return endpoint;
}

export function liveEndpoint(origin: string): string {
  const endpoint = validateEndpoint(origin);
  endpoint.protocol = endpoint.protocol === 'https:' || endpoint.protocol === 'wss:' ? 'wss:' : 'ws:';
  endpoint.pathname = '/v1/web/live';
  return endpoint.toString();
}
