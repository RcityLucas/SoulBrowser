import type { TaskStreamEvent } from './types.js';

export interface TaskEventStreamInit {
  url: URL;
  fetchFn: typeof fetch;
  headers?: Record<string, string>;
  lastEventId?: string;
  retryDelayMs?: number;
  maxRetryDelayMs?: number;
}

export type TaskEventListener = (event: TaskStreamEvent) => void;
export type TaskStreamErrorListener = (error: Error) => void;
export type TaskStreamConnectionListener = (connected: boolean) => void;

interface ParsedSseEvent {
  id?: string;
  event?: string;
  data?: string;
  retry?: number;
}

export class TaskEventStream {
  private readonly fetchFn: typeof fetch;
  private readonly url: URL;
  private readonly headers: Record<string, string>;
  private readonly initialRetryDelay: number;
  private readonly maxRetryDelay: number;
  private controller: AbortController | null = null;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private closed = false;
  private retryDelay: number;
  private lastEventId?: string;
  private eventListeners = new Set<TaskEventListener>();
  private errorListeners = new Set<TaskStreamErrorListener>();
  private connectionListeners = new Set<TaskStreamConnectionListener>();

  constructor(init: TaskEventStreamInit) {
    this.fetchFn = init.fetchFn;
    this.url = init.url;
    this.headers = init.headers ?? {};
    this.initialRetryDelay = Math.max(init.retryDelayMs ?? 1500, 250);
    this.maxRetryDelay = Math.max(init.maxRetryDelayMs ?? 15000, this.initialRetryDelay);
    this.retryDelay = this.initialRetryDelay;
    this.lastEventId = init.lastEventId;
    this.connect();
  }

  onEvent(listener: TaskEventListener): () => void {
    this.eventListeners.add(listener);
    return () => this.eventListeners.delete(listener);
  }

  onError(listener: TaskStreamErrorListener): () => void {
    this.errorListeners.add(listener);
    return () => this.errorListeners.delete(listener);
  }

  onConnectionChange(listener: TaskStreamConnectionListener): () => void {
    this.connectionListeners.add(listener);
    return () => this.connectionListeners.delete(listener);
  }

  close(): void {
    this.closed = true;
    if (this.controller) {
      this.controller.abort();
      this.controller = null;
    }
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
  }

  get lastId(): string | undefined {
    return this.lastEventId;
  }

  private async connect() {
    if (this.closed) {
      return;
    }

    if (this.controller) {
      this.controller.abort();
    }
    this.controller = new AbortController();

    const headers = new Headers(this.headers);
    headers.set('Accept', 'text/event-stream');
    if (this.lastEventId) {
      headers.set('Last-Event-ID', this.lastEventId);
    }

    try {
      const response = await this.fetchFn(this.url, {
        method: 'GET',
        headers,
        signal: this.controller.signal,
      });

      if (!response.ok || !response.body) {
        throw new Error(`SSE connection failed: ${response.status} ${response.statusText}`);
      }

      this.retryDelay = this.initialRetryDelay;
      this.notifyConnection(true);
      await this.consume(response.body);
    } catch (error) {
      if (this.closed) {
        return;
      }
      if ((error as any)?.name !== 'AbortError') {
        this.emitError(error as Error);
      }
      this.scheduleReconnect();
    }
  }

  private async consume(stream: ReadableStream<Uint8Array>) {
    const reader = stream.getReader();
    const decoder = new TextDecoder('utf-8');
    let buffer = '';

    try {
      while (!this.closed) {
        const { value, done } = await reader.read();
        if (done) {
          break;
        }
        buffer += decoder.decode(value, { stream: true });
        let boundary = buffer.indexOf('\n\n');
        while (boundary !== -1) {
          const chunk = buffer.slice(0, boundary);
          buffer = buffer.slice(boundary + 2);
          this.processChunk(chunk);
          boundary = buffer.indexOf('\n\n');
        }
      }
    } finally {
      try {
        reader.releaseLock();
      } catch (err) {
        // ignore
      }
    }

    if (!this.closed) {
      this.scheduleReconnect();
    }
  }

  private processChunk(chunk: string) {
    const trimmed = chunk.trim();
    if (!trimmed) {
      return;
    }
    const parsed = parseSseEvent(trimmed);
    if (!parsed) {
      return;
    }
    if (typeof parsed.retry === 'number' && Number.isFinite(parsed.retry) && parsed.retry > 0) {
      this.retryDelay = Math.min(parsed.retry, this.maxRetryDelay);
    }
    if (parsed.id) {
      this.lastEventId = parsed.id;
    }
    if (!parsed.data) {
      return;
    }

    const payload = parseEventPayload(parsed.data);
    if (payload && typeof payload === 'object' && 'event' in payload) {
      this.eventListeners.forEach((listener) => {
        try {
          listener(payload as TaskStreamEvent);
        } catch (err) {
          this.emitError(err as Error);
        }
      });
    }
  }

  private scheduleReconnect() {
    if (this.closed) {
      return;
    }
    this.notifyConnection(false);
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
    }
    const delay = this.retryDelay;
    this.retryDelay = Math.min(this.retryDelay * 2, this.maxRetryDelay);
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      this.connect();
    }, delay);
  }

  private emitError(error: Error) {
    this.errorListeners.forEach((listener) => {
      try {
        listener(error);
      } catch (err) {
        console.error('TaskEventStream error listener failed', err);
      }
    });
  }

  private notifyConnection(state: boolean) {
    this.connectionListeners.forEach((listener) => {
      try {
        listener(state);
      } catch (err) {
        console.error('TaskEventStream connection listener failed', err);
      }
    });
  }
}

function parseSseEvent(block: string): ParsedSseEvent | null {
  const event: ParsedSseEvent = {};
  const lines = block.split(/\r?\n/);
  for (const line of lines) {
    if (!line || line.startsWith(':')) {
      continue;
    }
    const separator = line.indexOf(':');
    let field = line;
    let value = '';
    if (separator !== -1) {
      field = line.slice(0, separator);
      value = line.slice(separator + 1);
      if (value.startsWith(' ')) {
        value = value.slice(1);
      }
    }
    switch (field) {
      case 'data':
        event.data = event.data ? `${event.data}\n${value}` : value;
        break;
      case 'id':
        event.id = value;
        break;
      case 'event':
        event.event = value;
        break;
      case 'retry': {
        const parsedRetry = Number.parseInt(value, 10);
        if (Number.isFinite(parsedRetry) && parsedRetry >= 0) {
          event.retry = parsedRetry;
        }
        break;
      }
      default:
        break;
    }
  }
  if (!event.data && !event.id && !event.event && typeof event.retry === 'undefined') {
    return null;
  }
  return event;
}

function parseEventPayload(raw: string): unknown {
  const trimmed = raw.trim();
  if (!trimmed) {
    return null;
  }
  try {
    return JSON.parse(trimmed);
  } catch {
    return { event: 'message', data: trimmed };
  }
}
