/**
 * WebSocket Client for real-time communication
 */

import type { WebSocketMessage, ClientMessage, ServerMessage, MessageType } from '@/types';

type EventListener<T = any> = (data: T) => void;

export class WebSocketClient {
  private ws: WebSocket | null = null;
  private url: string;
  private reconnectInterval: number = 3000;
  private reconnectAttempts: number = 0;
  private maxReconnectAttempts: number = 10;
  private heartbeatInterval: number = 30000;
  private heartbeatTimer: NodeJS.Timeout | null = null;
  private listeners: Map<string, Set<EventListener>> = new Map();
  private isManualClose: boolean = false;

  constructor(url: string = 'ws://localhost:8080/ws') {
    this.url = url;
  }

  connect(): Promise<void> {
    return new Promise((resolve, reject) => {
      try {
        this.ws = new WebSocket(this.url);

        this.ws.onopen = () => {
          console.log('[WebSocket] Connected');
          this.reconnectAttempts = 0;
          this.startHeartbeat();
          this.notifyListeners('connected', {});
          resolve();
        };

        this.ws.onmessage = (event) => {
          try {
            const message: WebSocketMessage = JSON.parse(event.data);
            console.log('[WebSocket] Received:', message.type);
            this.notifyListeners(message.type, message.payload);
          } catch (error) {
            console.error('[WebSocket] Failed to parse message:', error);
          }
        };

        this.ws.onerror = (error) => {
          console.error('[WebSocket] Error:', error);
          this.notifyListeners('error', error);
          reject(error);
        };

        this.ws.onclose = (event) => {
          console.log('[WebSocket] Closed:', event.code, event.reason);
          this.stopHeartbeat();
          this.notifyListeners('disconnected', {});

          if (!this.isManualClose && this.reconnectAttempts < this.maxReconnectAttempts) {
            this.reconnectAttempts++;
            console.log(`[WebSocket] Reconnecting... (${this.reconnectAttempts}/${this.maxReconnectAttempts})`);
            setTimeout(() => this.connect(), this.reconnectInterval);
          }
        };
      } catch (error) {
        reject(error);
      }
    });
  }

  disconnect(): void {
    this.isManualClose = true;
    this.stopHeartbeat();
    if (this.ws) {
      this.ws.close();
      this.ws = null;
    }
  }

  send<T extends ClientMessage>(message: T): void {
    if (this.ws?.readyState === WebSocket.OPEN) {
      const payload: WebSocketMessage = {
        type: message.type,
        payload: message.payload,
        timestamp: Date.now(),
      };
      this.ws.send(JSON.stringify(payload));
      console.log('[WebSocket] Sent:', message.type);
    } else {
      console.warn('[WebSocket] Cannot send message, connection not open');
    }
  }

  on<T = any>(eventType: string, callback: EventListener<T>): () => void {
    if (!this.listeners.has(eventType)) {
      this.listeners.set(eventType, new Set());
    }
    this.listeners.get(eventType)!.add(callback);

    // Return unsubscribe function
    return () => {
      const listeners = this.listeners.get(eventType);
      if (listeners) {
        listeners.delete(callback);
      }
    };
  }

  off(eventType: string, callback?: EventListener): void {
    if (!callback) {
      this.listeners.delete(eventType);
    } else {
      const listeners = this.listeners.get(eventType);
      if (listeners) {
        listeners.delete(callback);
      }
    }
  }

  private notifyListeners(eventType: string, data: any): void {
    const listeners = this.listeners.get(eventType);
    if (listeners) {
      listeners.forEach((callback) => {
        try {
          callback(data);
        } catch (error) {
          console.error(`[WebSocket] Error in listener for ${eventType}:`, error);
        }
      });
    }
  }

  private startHeartbeat(): void {
    this.stopHeartbeat();
    this.heartbeatTimer = setInterval(() => {
      this.send({ type: 'ping', payload: {} });
    }, this.heartbeatInterval);
  }

  private stopHeartbeat(): void {
    if (this.heartbeatTimer) {
      clearInterval(this.heartbeatTimer);
      this.heartbeatTimer = null;
    }
  }

  get isConnected(): boolean {
    return this.ws?.readyState === WebSocket.OPEN;
  }

  get readyState(): number {
    return this.ws?.readyState ?? WebSocket.CLOSED;
  }
}

// Singleton instance
let wsClient: WebSocketClient | null = null;

export function getWebSocketClient(): WebSocketClient {
  if (!wsClient) {
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const host = window.location.host;
    const url = `${protocol}//${host}/ws`;
    wsClient = new WebSocketClient(url);
  }
  return wsClient;
}

export default getWebSocketClient;
