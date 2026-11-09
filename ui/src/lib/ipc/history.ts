import type { HistoryReq } from './bindings/HistoryReq';
import type { HistoryResp } from './bindings/HistoryResp';
import { call } from './core';

export function ping(): Promise<HistoryResp> {
  const req: HistoryReq = { kind: 'ping' };
  return call<HistoryReq, HistoryResp>('ipc_history', req);
}
