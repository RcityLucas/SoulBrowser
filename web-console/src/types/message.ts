/**
 * WebSocket message type definitions
 */

export interface WebSocketMessage<T = any> {
  type: string;
  payload: T;
  timestamp: number;
  requestId?: string;
}

// Client → Server Messages
export type ClientMessage =
  | { type: 'ping'; payload: Record<string, never> }
  | { type: 'chat_message'; payload: ChatMessagePayload }
  | { type: 'task_start'; payload: { taskId: string } }
  | { type: 'task_pause'; payload: { taskId: string } }
  | { type: 'task_resume'; payload: { taskId: string } }
  | { type: 'task_cancel'; payload: { taskId: string } }
  | { type: 'subscribe_task'; payload: { taskId: string } }
  | { type: 'unsubscribe_task'; payload: { taskId: string } }
  | { type: 'subscribe_screenshot'; payload: { taskId: string; fps?: number } }
  | { type: 'unsubscribe_screenshot'; payload: { taskId: string } };

// Server → Client Messages
export type ServerMessage =
  | { type: 'pong'; payload: Record<string, never> }
  | { type: 'connected'; payload: ConnectionInfo }
  | { type: 'task_created'; payload: import('./task').Task }
  | { type: 'task_updated'; payload: import('./task').TaskUpdate }
  | { type: 'task_completed'; payload: TaskCompletedPayload }
  | { type: 'task_failed'; payload: TaskFailedPayload }
  | { type: 'screenshot'; payload: ScreenshotFrame }
  | { type: 'log_entry'; payload: LogEntry }
  | { type: 'chat_response'; payload: ChatResponsePayload }
  | { type: 'error'; payload: ErrorPayload };

export interface ChatMessagePayload {
  content: string;
  attachments?: any[];
}

export interface ChatResponsePayload {
  content: string;
  taskPlan?: import('./task').TaskPlan;
  suggestions?: string[];
}

export interface ConnectionInfo {
  sessionId: string;
  userId?: string;
  serverVersion: string;
  capabilities: string[];
}

export interface TaskCompletedPayload {
  taskId: string;
  result: any;
  duration: number;
  timestamp: Date;
}

export interface TaskFailedPayload {
  taskId: string;
  error: import('./task').TaskError;
  timestamp: Date;
}

export interface ScreenshotFrame {
  taskId: string;
  timestamp: Date;
  data: string; // base64 encoded image
  overlays: ElementOverlay[];
  viewport: Viewport;
}

export interface ElementOverlay {
  id: string;
  type: 'highlight' | 'label' | 'error';
  rect: DOMRect;
  label?: string;
  color?: string;
}

export interface DOMRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface Viewport {
  width: number;
  height: number;
  deviceScaleFactor: number;
}

export interface LogEntry {
  id: string;
  taskId: string;
  level: 'debug' | 'info' | 'warn' | 'error';
  message: string;
  timestamp: Date;
  metadata?: Record<string, any>;
}

export interface ErrorPayload {
  code: string;
  message: string;
  details?: string;
}

export enum MessageType {
  // Client messages
  PING = 'ping',
  CHAT_MESSAGE = 'chat_message',
  TASK_START = 'task_start',
  TASK_PAUSE = 'task_pause',
  TASK_RESUME = 'task_resume',
  TASK_CANCEL = 'task_cancel',
  SUBSCRIBE_TASK = 'subscribe_task',
  UNSUBSCRIBE_TASK = 'unsubscribe_task',
  SUBSCRIBE_SCREENSHOT = 'subscribe_screenshot',
  UNSUBSCRIBE_SCREENSHOT = 'unsubscribe_screenshot',

  // Server messages
  PONG = 'pong',
  CONNECTED = 'connected',
  TASK_CREATED = 'task_created',
  TASK_UPDATED = 'task_updated',
  TASK_COMPLETED = 'task_completed',
  TASK_FAILED = 'task_failed',
  SCREENSHOT = 'screenshot',
  LOG_ENTRY = 'log_entry',
  CHAT_RESPONSE = 'chat_response',
  ERROR = 'error',
}
