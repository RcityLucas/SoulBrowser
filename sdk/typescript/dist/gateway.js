export class SoulBrowserGatewayClient {
    constructor(options) {
        if (!options?.tenantId) {
            throw new Error('tenantId is required to call the gateway');
        }
        this.baseUrl = options.baseUrl ?? 'http://127.0.0.1:8710';
        this.tenantId = options.tenantId;
        this.fetchFn = options.fetchFn ?? globalThis.fetch;
        if (!this.fetchFn) {
            throw new Error('No fetch implementation available. Pass `fetchFn` when constructing the client.');
        }
        this.apiKey = options.apiKey;
        this.tenantToken = options.tenantToken;
        this.staticHeaders = options.headers ?? {};
    }
    setBaseUrl(url) {
        this.baseUrl = url;
    }
    setTenant(tenantId) {
        this.tenantId = tenantId;
    }
    configureAuth({ apiKey, tenantToken }) {
        this.apiKey = apiKey ?? this.apiKey;
        this.tenantToken = tenantToken ?? this.tenantToken;
    }
    async runTool(request, options) {
        if (!request?.tool) {
            throw new Error('`tool` is required when calling runTool');
        }
        const headers = this.buildHeaders(options);
        const response = await this.fetchFn(new URL('/v1/tools/run', this.baseUrl), {
            method: 'POST',
            headers,
            body: JSON.stringify(request),
        });
        if (!response.ok) {
            const text = await response.text().catch(() => '');
            throw new Error(`Gateway request failed: ${response.status} ${response.statusText} ${text}`);
        }
        return (await response.json());
    }
    buildHeaders(options) {
        const headers = {
            'content-type': 'application/json',
            'x-tenant-id': options?.tenantId ?? this.tenantId,
            ...this.staticHeaders,
            ...(options?.headers ?? {}),
        };
        if (this.apiKey && !headers.authorization) {
            headers.authorization = `Bearer ${this.apiKey}`;
        }
        if (this.tenantToken && !headers['x-tenant-token']) {
            headers['x-tenant-token'] = this.tenantToken;
        }
        return headers;
    }
}
