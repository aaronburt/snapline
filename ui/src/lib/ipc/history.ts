import type { Cursor } from './bindings/Cursor';
import type { HistoryCaptureDto } from './bindings/HistoryCaptureDto';
import type { HistoryFilter } from './bindings/HistoryFilter';
import type { HistoryPage } from './bindings/HistoryPage';
import type { HistoryReq } from './bindings/HistoryReq';
import type { HistoryResp } from './bindings/HistoryResp';
import type { HistoryStats } from './bindings/HistoryStats';
import type { IpcError } from './core';
import { call } from './core';

function unexpected(): IpcError {
  return {
    code: 'internal',
    message: 'Snapline returned an unexpected History answer.',
  };
}

function pageOf(resp: HistoryResp): HistoryPage {
  if (resp.kind === 'page' || resp.kind === 'search') {
    return resp.data.page;
  }
  throw unexpected();
}

export function ping(): Promise<HistoryResp> {
  const req: HistoryReq = { kind: 'ping' };
  return call<HistoryReq, HistoryResp>('ipc_history', req);
}

export async function historyPage(
  filter: HistoryFilter,
  cursor: Cursor | null,
): Promise<HistoryPage> {
  const req: HistoryReq = { kind: 'page', data: { filter, cursor } };
  return pageOf(await call<HistoryReq, HistoryResp>('ipc_history', req));
}

export async function historySearch(
  text: string,
  filter: HistoryFilter,
  cursor: Cursor | null,
): Promise<HistoryPage> {
  const req: HistoryReq = { kind: 'search', data: { text, filter, cursor } };
  return pageOf(await call<HistoryReq, HistoryResp>('ipc_history', req));
}

export async function historyGet(id: number): Promise<HistoryCaptureDto> {
  const req: HistoryReq = { kind: 'get', data: { id } };
  const resp = await call<HistoryReq, HistoryResp>('ipc_history', req);
  if (resp.kind === 'get') {
    return resp.data.capture;
  }
  throw unexpected();
}

export async function historyStats(): Promise<HistoryStats> {
  const req: HistoryReq = { kind: 'stats' };
  const resp = await call<HistoryReq, HistoryResp>('ipc_history', req);
  if (resp.kind === 'stats') {
    return resp.data.stats;
  }
  throw unexpected();
}

export async function historyConfirmDelete(id: number): Promise<boolean> {
  const req: HistoryReq = { kind: 'confirmDelete', data: { id } };
  const resp = await call<HistoryReq, HistoryResp>('ipc_history', req);
  if (resp.kind === 'confirmDelete') {
    return resp.data.confirmed;
  }
  throw unexpected();
}

export async function historyDelete(id: number): Promise<void> {
  const req: HistoryReq = { kind: 'delete', data: { id } };
  const resp = await call<HistoryReq, HistoryResp>('ipc_history', req);
  if (resp.kind !== 'delete') {
    throw unexpected();
  }
}

export async function historyDeleteMany(
  ids: number[],
): Promise<{ removed: number[]; failed: number }> {
  const req: HistoryReq = { kind: 'deleteMany', data: { ids } };
  const resp = await call<HistoryReq, HistoryResp>('ipc_history', req);
  if (resp.kind === 'deleteMany') {
    return resp.data;
  }
  throw unexpected();
}

export async function historyCopyImage(id: number): Promise<void> {
  const req: HistoryReq = { kind: 'copyImage', data: { id } };
  const resp = await call<HistoryReq, HistoryResp>('ipc_history', req);
  if (resp.kind !== 'copyImage') {
    throw unexpected();
  }
}

export async function historyShowInFolder(id: number): Promise<void> {
  const req: HistoryReq = { kind: 'showInFolder', data: { id } };
  const resp = await call<HistoryReq, HistoryResp>('ipc_history', req);
  if (resp.kind !== 'showInFolder') {
    throw unexpected();
  }
}

export async function historyOpenFile(id: number): Promise<void> {
  const req: HistoryReq = { kind: 'openFile', data: { id } };
  const resp = await call<HistoryReq, HistoryResp>('ipc_history', req);
  if (resp.kind !== 'openFile') {
    throw unexpected();
  }
}

export async function historyOpenUrl(id: number): Promise<void> {
  const req: HistoryReq = { kind: 'openUrl', data: { id } };
  const resp = await call<HistoryReq, HistoryResp>('ipc_history', req);
  if (resp.kind !== 'openUrl') {
    throw unexpected();
  }
}
