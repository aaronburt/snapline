import type { HotkeysReq } from './bindings/HotkeysReq';
import type { HotkeysResp } from './bindings/HotkeysResp';
import { call } from './core';

export function ping(): Promise<HotkeysResp> {
  const req: HotkeysReq = { kind: 'ping' };
  return call<HotkeysReq, HotkeysResp>('ipc_hotkeys', req);
}
