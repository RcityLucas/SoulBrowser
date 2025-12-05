export class SoulBrowserClient {
    constructor(options) {
        this.baseUrl = options?.baseUrl ?? 'http://127.0.0.1:8801';
        this.fetchFn = options?.fetchFn ?? globalThis.fetch;
        this.WebSocketCtor = options?.WebSocketClass ?? globalThis.WebSocket;
        if (!this.fetchFn) {
            throw new Error('No fetch implementation available. Pass `fetchFn` in the constructor.');
        }
    }
    setBaseUrl(url) {
        this.baseUrl = url;
    }
    getBaseUrl() {
        return this.baseUrl;
    }
    async chat(payload) {
        return this.post('/api/chat', payload);
    }
    async perceive(payload) {
        return this.post('/api/perceive', payload);
    }
    async createTask(payload) {
        return this.post('/api/tasks', payload);
    }
    async listTasks(limit) {
        const params = limit ? `?limit=${limit}` : '';
        const data = await this.get(`/api/tasks${params}`);
        return data.tasks;
    }
    async getTask(taskId, limit) {
        const params = limit ? `?limit=${limit}` : '';
        return this.get(`/api/tasks/${taskId}${params}`);
    }
    async getTaskStatus(taskId) {
        const data = await this.get(`/api/tasks/${taskId}/status`);
        return data.status;
    }
    async getTaskLogs(taskId, since) {
        const params = since ? `?since=${encodeURIComponent(since)}` : '';
        const data = await this.get(`/api/tasks/${taskId}/logs${params}`);
        return data.logs;
    }
    async getTaskObservations(taskId, limit) {
        const params = limit ? `?limit=${limit}` : '';
        return this.get(`/api/tasks/${taskId}/observations${params}`);
    }
    async listRecordings(limit, state) {
        const params = new URLSearchParams();
        if (typeof limit === 'number')
            params.set('limit', String(limit));
        if (state)
            params.set('state', state);
        const query = params.toString();
        const suffix = query.length ? `?${query}` : '';
        return this.get(`/api/recordings${suffix}`);
    }
    async getRecording(sessionId) {
        return this.get(`/api/recordings/${sessionId}`);
    }
    async getTaskArtifacts(taskId) {
        return this.get(`/api/tasks/${taskId}/artifacts`);
    }
    async getTaskAnnotations(taskId) {
        return this.get(`/api/tasks/${taskId}/annotations`);
    }
    async createTaskAnnotation(taskId, payload) {
        return this.post(`/api/tasks/${taskId}/annotations`, payload);
    }
    async executeTask(taskId, payload) {
        return this.post(`/api/tasks/${taskId}/execute`, payload ?? {});
    }
    async cancelTask(taskId, reason) {
        return this.post(`/api/tasks/${taskId}/cancel`, { reason });
    }
    openTaskStream(taskId, options) {
        if (!this.WebSocketCtor) {
            throw new Error('No WebSocket implementation is available. Pass `WebSocketClass` in options.');
        }
        const base = new URL(this.baseUrl);
        base.protocol = base.protocol === 'https:' ? 'wss:' : 'ws:';
        const defaultPath = options?.viaGateway
            ? `/v1/tasks/${taskId}/stream`
            : `/api/tasks/${taskId}/stream`;
        const customPath = options?.customPath;
        base.pathname = customPath ? customPath.replace(':task_id', taskId) : defaultPath;
        base.search = '';
        base.hash = '';
        return new this.WebSocketCtor(base.toString());
    }
    async get(path) {
        const response = await this.fetchFn(new URL(path, this.baseUrl), {
            method: 'GET',
            headers: {
                'Content-Type': 'application/json',
            },
        });
        return this.handleResponse(response);
    }
    async post(path, body) {
        const response = await this.fetchFn(new URL(path, this.baseUrl), {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json',
            },
            body: JSON.stringify(body ?? {}),
        });
        return this.handleResponse(response);
    }
    async handleResponse(response) {
        if (!response.ok) {
            const text = await response.text().catch(() => '');
            throw new Error(`Request failed: ${response.status} ${response.statusText} ${text}`);
        }
        return (await response.json());
    }
}
