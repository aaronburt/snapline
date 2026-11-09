import type { AppReq } from '../../lib/ipc/bindings/AppReq';
import type { AppResp } from '../../lib/ipc/bindings/AppResp';
import { call } from '../../lib/ipc/core';

export function panelResize(height: number): Promise<AppResp> {
  const req: AppReq = { kind: 'panelResize', data: { height } };
  return call<AppReq, AppResp>('ipc_app', req);
}

export function panelClose(): Promise<AppResp> {
  const req: AppReq = { kind: 'panelClose' };
  return call<AppReq, AppResp>('ipc_app', req);
}

export function panelAction(action: string): Promise<AppResp> {
  const req: AppReq = { kind: 'panelAction', data: { action } };
  return call<AppReq, AppResp>('ipc_app', req);
}

export function panelCopyLink(): Promise<AppResp> {
  const req: AppReq = { kind: 'panelCopyLink' };
  return call<AppReq, AppResp>('ipc_app', req);
}

export function openMain(route: string): Promise<AppResp> {
  const req: AppReq = { kind: 'openMain', data: { route } };
  return call<AppReq, AppResp>('ipc_app', req);
}

export function openCapture(id: number): Promise<AppResp> {
  const req: AppReq = { kind: 'openCapture', data: { id } };
  return call<AppReq, AppResp>('ipc_app', req);
}
