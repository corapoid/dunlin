import { acceptSnapshot, decodeCatalog } from './catalog.ts';
import type { CatalogSnapshot } from './catalog.ts';
import { liveEndpoint } from './transport.ts';

export type LiveStatus = 'connecting' | 'live' | 'reconnecting' | 'offline' | 'error' | 'authorization_required';

/** Read-only catalog client. Reconnect never repeats prompts or mutations. */
export class CatalogClient {
  private socket: WebSocket | null = null;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private stopped = true;
  private attempts = 0;
  private selected: string | null = null;
  private previous: CatalogSnapshot | null = null;
  private readonly origin: string;
  private readonly onSnapshot: (snapshot: CatalogSnapshot) => void;
  private readonly onStatus: (status: LiveStatus) => void;
  private readonly makeSocket: (url: string) => WebSocket;

  constructor(
    origin: string,
    onSnapshot: (snapshot: CatalogSnapshot) => void,
    onStatus: (status: LiveStatus) => void,
    makeSocket: (url: string) => WebSocket = (url) => new WebSocket(url),
  ) {
    this.origin = origin;
    this.onSnapshot = onSnapshot;
    this.onStatus = onStatus;
    this.makeSocket = makeSocket;
  }

  start(): void {
    this.stop();
    this.stopped = false;
    this.attempts = 0;
    this.connect();
  }

  stop(): void {
    this.stopped = true;
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = null;
    const socket = this.socket;
    this.socket = null;
    socket?.close();
  }

  selectProject(project: string | null): void {
    this.selected = project;
    this.subscribe();
  }

  private subscribe(): void {
    if (this.socket?.readyState === 1) {
      this.socket.send(JSON.stringify({ type: 'subscribe', protocol_version: 1, project_id: this.selected }));
    }
  }

  private connect(): void {
    if (this.stopped) return;
    this.onStatus(this.attempts ? 'reconnecting' : 'connecting');
    this.previous = null;
    const socket = this.makeSocket(liveEndpoint(this.origin));
    this.socket = socket;
    socket.onopen = () => { if (this.socket === socket && !this.stopped) this.subscribe(); };
    socket.onmessage = (message) => {
      if (this.socket !== socket || this.stopped) return;
      try {
        if (typeof message.data !== 'string') throw new Error('Unsupported frame');
        const event = decodeCatalog(message.data);
        if (event.type === 'resync_required') {
          this.onStatus('reconnecting');
          // The server follows with an authoritative snapshot; don't replay any action.
          return;
        }
        if (event.type === 'error') {
          this.stop();
          this.onStatus(event.code === 'authorization_required' ? 'authorization_required' : 'error');
          return;
        }
        if (event.snapshot.project_id !== this.selected || !acceptSnapshot(this.previous, event.snapshot)) return;
        this.previous = event.snapshot;
        this.attempts = 0;
        this.onSnapshot(event.snapshot);
        this.onStatus('live');
      } catch {
        this.stop();
        this.onStatus('error');
      }
    };
    socket.onclose = () => {
      if (this.socket !== socket || this.stopped) return;
      this.socket = null;
      if (this.attempts >= 8) { this.onStatus('offline'); return; }
      this.onStatus('reconnecting');
      const delay = Math.min(15_000, 500 * 2 ** this.attempts++) + Math.random() * 250;
      this.timer = setTimeout(() => { this.timer = null; this.connect(); }, delay);
    };
    socket.onerror = () => { /* onclose owns bounded backoff; no duplicate reconnect. */ };
  }
}
