export type SessionStatus = 'initializing' | 'active' | 'idle' | 'completed' | 'failed';

export interface SessionRecord {
  id: string;
  tenant_id: string;
  subject_id?: string;
  profile_id?: string;
  profile_label?: string;
  status: SessionStatus;
  created_at: string;
  updated_at: string;
  last_event_at?: string;
  last_task_id?: string;
  live_path: string;
  share_token?: string;
  share_url?: string;
  share_path?: string;
}

export interface LiveOverlayEntry {
  session_id: string;
  recorded_at: string;
  task_id?: string;
  source: string;
  data: Record<string, any>;
}

export interface RouteSummary {
  session: string;
  page?: string;
  frame?: string;
}

export interface LiveFramePayload {
  session_id: string;
  task_id?: string;
  recorded_at: string;
  screenshot_base64: string;
  route?: RouteSummary;
  overlays?: LiveOverlayEntry[];
}

export interface SessionSnapshot {
  session: SessionRecord;
  overlays: LiveOverlayEntry[];
  last_frame?: LiveFramePayload | null;
}

export type SessionLiveEvent =
  | { type: 'snapshot'; snapshot: SessionSnapshot }
  | { type: 'status'; session_id: string; status: SessionStatus }
  | { type: 'frame'; frame: LiveFramePayload }
  | { type: 'overlay'; overlay: LiveOverlayEntry };

export interface SessionShareContext {
  session_id: string;
  live_path: string;
  share_token?: string;
  share_url?: string;
}

export interface CreateSessionRequest {
  profile_id?: string;
  profile_label?: string;
  description?: string;
  shared?: boolean;
}
