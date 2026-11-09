import type { UploadReq } from './bindings/UploadReq';
import type { UploadResp } from './bindings/UploadResp';
import { call } from './core';

export function ping(): Promise<UploadResp> {
  const req: UploadReq = { kind: 'ping' };
  return call<UploadReq, UploadResp>('ipc_upload', req);
}
