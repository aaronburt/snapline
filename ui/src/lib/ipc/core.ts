import { invoke } from '@tauri-apps/api/core';

export interface IpcError {
  code: string;
  message: string;
}

function isIpcError(value: unknown): value is IpcError {
  if (typeof value !== 'object' || value === null) {
    return false;
  }
  const candidate = value as Record<string, unknown>;
  return typeof candidate.code === 'string' && typeof candidate.message === 'string';
}

export async function call<Req, Resp>(command: string, req: Req): Promise<Resp> {
  try {
    return await invoke<Resp>(command, { req });
  } catch (error) {
    if (isIpcError(error)) {
      throw error;
    }
    throw {
      code: 'internal',
      message: typeof error === 'string' ? error : 'The request could not be completed.',
    } satisfies IpcError;
  }
}
