import type { RecordReq } from './bindings/RecordReq';
import type { RecordResp } from './bindings/RecordResp';
import { call } from './core';

export function ping(): Promise<RecordResp> {
  const req: RecordReq = { kind: 'ping' };
  return call<RecordReq, RecordResp>('ipc_record', req);
}
