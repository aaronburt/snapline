import type { AppReq } from './bindings/AppReq';
import type { AppResp } from './bindings/AppResp';
import { call } from './core';

export function ping(): Promise<AppResp> {
  const req: AppReq = { kind: 'ping' };
  return call<AppReq, AppResp>('ipc_app', req);
}

export function ready(): Promise<AppResp> {
  const req: AppReq = { kind: 'ready' };
  return call<AppReq, AppResp>('ipc_app', req);
}

export function info(): Promise<AppResp> {
  const req: AppReq = { kind: 'info' };
  return call<AppReq, AppResp>('ipc_app', req);
}
