import type { AppReq } from '../../lib/ipc/bindings/AppReq';
import type { AppResp } from '../../lib/ipc/bindings/AppResp';
import type { RecentCaptureDto } from '../../lib/ipc/bindings/RecentCaptureDto';
import type { IpcError } from '../../lib/ipc/core';
import { call } from '../../lib/ipc/core';

function unexpected(): IpcError {
  return {
    code: 'internal',
    message: 'Snapline returned an unexpected answer.',
  };
}

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

export async function panelRecent(): Promise<RecentCaptureDto[]> {
  const req: AppReq = { kind: 'recent' };
  const resp = await call<AppReq, AppResp>('ipc_app', req);
  if (resp.kind === 'recent') {
    return resp.data.captures;
  }
  throw unexpected();
}
