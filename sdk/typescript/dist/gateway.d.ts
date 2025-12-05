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
export declare class SoulBrowserGatewayClient {
    private baseUrl;
    private tenantId;
    private fetchFn;
    private apiKey?;
    private tenantToken?;
    private staticHeaders;
    constructor(options: GatewayClientOptions);
    setBaseUrl(url: string): void;
    setTenant(tenantId: string): void;
    configureAuth({ apiKey, tenantToken }: {
        apiKey?: string;
        tenantToken?: string;
    }): void;
    runTool(request: GatewayToolRunRequest, options?: RunToolOptions): Promise<GatewayToolRunResponse>;
    private buildHeaders;
}
