import type { RecordKindDto } from './bindings/RecordKindDto';
import type { RecordReq } from './bindings/RecordReq';
import type { RecordResp } from './bindings/RecordResp';
import type { RecordStatusDto } from './bindings/RecordStatusDto';
import { call } from './core';

function statusOf(response: RecordResp): RecordStatusDto {
  if (response.kind === 'status') {
    return response.data.status;
  }
  throw new Error('Unexpected record response');
}

async function request(req: RecordReq): Promise<RecordStatusDto> {
  return statusOf(await call<RecordReq, RecordResp>('ipc_record', req));
}

export function start(kind: RecordKindDto): Promise<RecordStatusDto> {
  const req: RecordReq = { kind: 'start', data: { kind } };
  return request(req);
}

export function startVideo(): Promise<RecordStatusDto> {
  return start('video');
}

export function startGif(): Promise<RecordStatusDto> {
  return start('gif');
}

export function startWindow(): Promise<RecordStatusDto> {
  return start('window');
}

export function stop(): Promise<RecordStatusDto> {
  const req: RecordReq = { kind: 'stop' };
  return request(req);
}

export function state(): Promise<RecordStatusDto> {
  const req: RecordReq = { kind: 'state' };
  return request(req);
}

export function ping(): Promise<RecordResp> {
  const req: RecordReq = { kind: 'ping' };
  return call<RecordReq, RecordResp>('ipc_record', req);
}
