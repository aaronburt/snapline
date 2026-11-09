import type { BindingDto } from './bindings/BindingDto';
import type { BlockedDto } from './bindings/BlockedDto';
import type { HotkeysReq } from './bindings/HotkeysReq';
import type { HotkeysResp } from './bindings/HotkeysResp';
import type { PreviewDto } from './bindings/PreviewDto';
import type { IpcError } from './core';
import { call } from './core';

export interface HotkeysState {
  bindings: BindingDto[];
  blocked: BlockedDto[];
  paused: boolean;
}

function unexpected(): IpcError {
  return {
    code: 'internal',
    message: 'Snapline returned an unexpected hotkeys answer.',
  };
}

export function ping(): Promise<HotkeysResp> {
  const req: HotkeysReq = { kind: 'ping' };
  return call<HotkeysReq, HotkeysResp>('ipc_hotkeys', req);
}

export async function get(): Promise<HotkeysState> {
  const req: HotkeysReq = { kind: 'get' };
  const resp = await call<HotkeysReq, HotkeysResp>('ipc_hotkeys', req);
  if (resp.kind === 'state') {
    return resp.data;
  }
  throw unexpected();
}

export async function preview(owner: string, chord: string): Promise<PreviewDto> {
  const req: HotkeysReq = { kind: 'preview', data: { owner, chord } };
  const resp = await call<HotkeysReq, HotkeysResp>('ipc_hotkeys', req);
  if (resp.kind === 'preview') {
    return resp.data.preview;
  }
  throw unexpected();
}

export async function pause(): Promise<boolean> {
  const req: HotkeysReq = { kind: 'pause' };
  const resp = await call<HotkeysReq, HotkeysResp>('ipc_hotkeys', req);
  if (resp.kind === 'paused') {
    return resp.data.paused;
  }
  throw unexpected();
}

export async function resume(): Promise<boolean> {
  const req: HotkeysReq = { kind: 'resume' };
  const resp = await call<HotkeysReq, HotkeysResp>('ipc_hotkeys', req);
  if (resp.kind === 'paused') {
    return resp.data.paused;
  }
  throw unexpected();
}
