import type { IpcError } from './core';
import type { SettingsDto } from './bindings/SettingsDto';
import type { SettingsReq } from './bindings/SettingsReq';
import type { SettingsResp } from './bindings/SettingsResp';
import { call } from './core';

function unexpected(): IpcError {
  return {
    code: 'internal',
    message: 'Snapline returned an unexpected settings answer.',
  };
}

function settingsOf(resp: SettingsResp): SettingsDto {
  if (resp.kind === 'settings' || resp.kind === 'updated') {
    return resp.data.settings;
  }
  throw unexpected();
}

export function ping(): Promise<SettingsResp> {
  const req: SettingsReq = { kind: 'ping' };
  return call<SettingsReq, SettingsResp>('ipc_settings', req);
}

export async function get(): Promise<SettingsDto> {
  const req: SettingsReq = { kind: 'get' };
  return settingsOf(await call<SettingsReq, SettingsResp>('ipc_settings', req));
}

export async function update(patch: Record<string, unknown>): Promise<SettingsDto> {
  const req: SettingsReq = { kind: 'update', data: { patch } };
  return settingsOf(await call<SettingsReq, SettingsResp>('ipc_settings', req));
}

export async function previewName(pattern: string, format: string): Promise<string> {
  const req: SettingsReq = { kind: 'previewName', data: { pattern, format } };
  const resp = await call<SettingsReq, SettingsResp>('ipc_settings', req);
  if (resp.kind === 'previewName') {
    return resp.data.name;
  }
  throw unexpected();
}

export async function browseFolder(current: string | null): Promise<string | null> {
  const req: SettingsReq = { kind: 'browseFolder', data: { current } };
  const resp = await call<SettingsReq, SettingsResp>('ipc_settings', req);
  if (resp.kind === 'browseFolder') {
    return resp.data.path;
  }
  throw unexpected();
}
