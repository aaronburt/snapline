import type { ToolReq } from './bindings/ToolReq';
import type { ToolResp } from './bindings/ToolResp';
import { call } from './core';

export function ping(): Promise<ToolResp> {
  const req: ToolReq = { kind: 'ping' };
  return call<ToolReq, ToolResp>('ipc_tool', req);
}
