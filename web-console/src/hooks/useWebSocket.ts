/**
 * WebSocket hook
 */

import { useEffect, useRef, useCallback } from 'react';
import { getWebSocketClient } from '@/api/websocket';
import type { ClientMessage } from '@/types';

export function useWebSocket() {
  const wsRef = useRef(getWebSocketClient());
  const isConnected = useRef(false);

  useEffect(() => {
    const ws = wsRef.current;

    if (!isConnected.current) {
      ws.connect().then(() => {
        isConnected.current = true;
      }).catch((error) => {
        console.error('Failed to connect WebSocket:', error);
      });
    }

    return () => {
      // Don't disconnect on unmount, keep connection alive
    };
  }, []);

  const send = useCallback(<T extends ClientMessage>(message: T) => {
    wsRef.current.send(message);
  }, []);

  const on = useCallback(<T = any>(eventType: string, callback: (data: T) => void) => {
    return wsRef.current.on(eventType, callback);
  }, []);

  return {
    send,
    on,
    client: wsRef.current,
  };
}

export default useWebSocket;
