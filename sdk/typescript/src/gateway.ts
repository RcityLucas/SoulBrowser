import type { GatewayToolRunRequest, GatewayToolRunResponse } from './types.js';

export interface GatewayClientOptions {
  baseUrl?: string;
  tenantId: string;
  fetchFn?: typeof fetch;
  apiKey?: string;
  tenantToken?: string;
  headers?: Record<string, string>;
}

export interface RunToolOptions {
  tenantId?: string;
  headers?: Record<string, string>;
}

export class SoulBrowserGatewayClient {
  private baseUrl: string;
  private tenantId: string;
  private fetchFn: typeof fetch;
  private apiKey?: string;
  private tenantToken?: string;
  private staticHeaders: Record<string, string>;

  constructor(options: GatewayClientOptions) {
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

  setBaseUrl(url: string) {
    this.baseUrl = url;
  }

  setTenant(tenantId: string) {
    this.tenantId = tenantId;
  }

  configureAuth({ apiKey, tenantToken }: { apiKey?: string; tenantToken?: string }) {
    this.apiKey = apiKey ?? this.apiKey;
    this.tenantToken = tenantToken ?? this.tenantToken;
  }

  async runTool(
    request: GatewayToolRunRequest,
    options?: RunToolOptions
  ): Promise<GatewayToolRunResponse> {
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
    return (await response.json()) as GatewayToolRunResponse;
  }

  private buildHeaders(options?: RunToolOptions): Record<string, string> {
    const headers: Record<string, string> = {
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
