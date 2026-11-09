import type { ConvertFormatDto } from './bindings/ConvertFormatDto';
import type { ConvertProgressDto } from './bindings/ConvertProgressDto';
import type { ConvertSource } from './bindings/ConvertSource';
import type { ConvertedDto } from './bindings/ConvertedDto';
import type { EditorOutcomeDto } from './bindings/EditorOutcomeDto';
import type { ToolReq } from './bindings/ToolReq';
import type { ToolResp } from './bindings/ToolResp';
import type { IpcError } from './core';
import { call } from './core';

export interface QrMade {
  svg: string;
  link: string | null;
}

export interface QrScanned {
  text: string;
  link: string | null;
  isUrl: boolean;
}

export type PinSource = 'file' | 'clipboard';

type PinRequest = { kind: 'pinFile' } | { kind: 'pinClipboard' } | { kind: 'pinMenu' };

type PinResponse = { kind: 'pinned'; data: { source: PinSource } } | { kind: 'pinCancelled' };

function unexpected(): IpcError {
  return {
    code: 'internal',
    message: 'Snapline returned an unexpected Tools answer.',
  };
}

export function ping(): Promise<ToolResp> {
  const req: ToolReq = { kind: 'ping' };
  return call<ToolReq, ToolResp>('ipc_tool', req);
}

export async function toolAvailable(tool: string): Promise<boolean> {
  const req: ToolReq = { kind: 'available', data: { tool } };
  const resp = await call<ToolReq, ToolResp>('ipc_tool', req);
  if (resp.kind === 'available') {
    return resp.data.available;
  }
  throw unexpected();
}

export async function editorOpenHistory(captureId: number): Promise<EditorOutcomeDto> {
  const req: ToolReq = { kind: 'editorOpenHistory', data: { captureId } };
  const resp = await call<ToolReq, ToolResp>('ipc_tool', req);
  if (resp.kind === 'editorOpened') {
    return resp.data.outcome;
  }
  throw unexpected();
}

export async function editorOpenFile(): Promise<EditorOutcomeDto> {
  const req: ToolReq = { kind: 'editorOpenFile' };
  const resp = await call<ToolReq, ToolResp>('ipc_tool', req);
  if (resp.kind === 'editorOpened') {
    return resp.data.outcome;
  }
  throw unexpected();
}

export async function editorOpenClipboard(): Promise<EditorOutcomeDto> {
  const req: ToolReq = { kind: 'editorOpenClipboard' };
  const resp = await call<ToolReq, ToolResp>('ipc_tool', req);
  if (resp.kind === 'editorOpened') {
    return resp.data.outcome;
  }
  throw unexpected();
}

export async function pinFromFile(): Promise<PinSource | null> {
  const req: PinRequest = { kind: 'pinFile' };
  const resp = await call<PinRequest, PinResponse>('ipc_tool', req);
  if (resp.kind === 'pinned') {
    return resp.data.source;
  }
  if (resp.kind === 'pinCancelled') {
    return null;
  }
  throw unexpected();
}

export async function pinFromClipboard(): Promise<PinSource | null> {
  const req: PinRequest = { kind: 'pinClipboard' };
  const resp = await call<PinRequest, PinResponse>('ipc_tool', req);
  if (resp.kind === 'pinned') {
    return resp.data.source;
  }
  if (resp.kind === 'pinCancelled') {
    return null;
  }
  throw unexpected();
}

export async function pinMenu(): Promise<PinSource | null> {
  const req: PinRequest = { kind: 'pinMenu' };
  const resp = await call<PinRequest, PinResponse>('ipc_tool', req);
  if (resp.kind === 'pinned') {
    return resp.data.source;
  }
  if (resp.kind === 'pinCancelled') {
    return null;
  }
  throw unexpected();
}

export async function qrMake(text: string): Promise<QrMade> {
  const req: ToolReq = { kind: 'qrMake', data: { text } };
  const resp = await call<ToolReq, ToolResp>('ipc_tool', req);
  if (resp.kind === 'qrMade') {
    return resp.data;
  }
  throw unexpected();
}

export async function qrScan(captureId: number | null = null): Promise<QrScanned> {
  const req: ToolReq = { kind: 'qrScan', data: { captureId } };
  const resp = await call<ToolReq, ToolResp>('ipc_tool', req);
  if (resp.kind === 'qrScanned') {
    return resp.data;
  }
  throw unexpected();
}

export async function qrCopyText(text: string): Promise<void> {
  const req: ToolReq = { kind: 'qrCopyText', data: { text } };
  const resp = await call<ToolReq, ToolResp>('ipc_tool', req);
  if (resp.kind !== 'qrCopied') {
    throw unexpected();
  }
}

export async function qrCopyImage(text: string): Promise<void> {
  const req: ToolReq = { kind: 'qrCopyImage', data: { text } };
  const resp = await call<ToolReq, ToolResp>('ipc_tool', req);
  if (resp.kind !== 'qrCopied') {
    throw unexpected();
  }
}

export async function qrSavePng(text: string): Promise<string> {
  const req: ToolReq = { kind: 'qrSavePng', data: { text } };
  const resp = await call<ToolReq, ToolResp>('ipc_tool', req);
  if (resp.kind === 'qrSaved') {
    return resp.data.path;
  }
  throw unexpected();
}

export async function qrOpen(link: string): Promise<void> {
  const req: ToolReq = { kind: 'qrOpen', data: { link } };
  const resp = await call<ToolReq, ToolResp>('ipc_tool', req);
  if (resp.kind !== 'qrOpened') {
    throw unexpected();
  }
}

export async function convertPickFile(): Promise<string | null> {
  const req: ToolReq = { kind: 'convertPickFile' };
  const resp = await call<ToolReq, ToolResp>('ipc_tool', req);
  if (resp.kind === 'convertFilePicked') {
    return resp.data.path;
  }
  throw unexpected();
}

export async function convertStart(
  source: ConvertSource,
  format: ConvertFormatDto,
): Promise<number> {
  const req: ToolReq = { kind: 'convertStart', data: { source, format } };
  const resp = await call<ToolReq, ToolResp>('ipc_tool', req);
  if (resp.kind === 'convertStarted') {
    return resp.data.jobId;
  }
  throw unexpected();
}

export async function convertProgress(jobId: number): Promise<ConvertProgressDto> {
  const req: ToolReq = { kind: 'convertProgress', data: { jobId } };
  const resp = await call<ToolReq, ToolResp>('ipc_tool', req);
  if (resp.kind === 'convertProgressed') {
    return resp.data.progress;
  }
  throw unexpected();
}

export async function convertCancel(jobId: number): Promise<void> {
  const req: ToolReq = { kind: 'convertCancel', data: { jobId } };
  const resp = await call<ToolReq, ToolResp>('ipc_tool', req);
  if (resp.kind !== 'convertCancelled') {
    throw unexpected();
  }
}

export async function convertResult(jobId: number): Promise<ConvertedDto> {
  const req: ToolReq = { kind: 'convertResult', data: { jobId } };
  const resp = await call<ToolReq, ToolResp>('ipc_tool', req);
  if (resp.kind === 'converted') {
    return resp.data.converted;
  }
  throw unexpected();
}

export async function pinHistory(captureId: number): Promise<void> {
  const resp = await call<{ kind: 'pinHistory'; data: { captureId: number } }, PinResponse>('ipc_tool', { kind: 'pinHistory', data: { captureId } });
  if (resp.kind !== 'pinned') throw new Error('The capture could not be pinned.');
}
