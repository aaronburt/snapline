import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));

import { pinFromClipboard, pinFromFile } from './tool';

beforeEach(() => {
  mocks.invoke.mockReset();
});

describe('pin ipc', () => {
  it('asks to pin a file and returns its source', async () => {
    mocks.invoke.mockResolvedValue({ kind: 'pinned', data: { source: 'file' } });
    await expect(pinFromFile()).resolves.toBe('file');
    expect(mocks.invoke).toHaveBeenCalledWith('ipc_tool', { req: { kind: 'pinFile' } });
  });

  it('asks to pin the clipboard and returns its source', async () => {
    mocks.invoke.mockResolvedValue({ kind: 'pinned', data: { source: 'clipboard' } });
    await expect(pinFromClipboard()).resolves.toBe('clipboard');
    expect(mocks.invoke).toHaveBeenCalledWith('ipc_tool', { req: { kind: 'pinClipboard' } });
  });

  it('returns null when the pin is cancelled', async () => {
    mocks.invoke.mockResolvedValue({ kind: 'pinCancelled' });
    await expect(pinFromFile()).resolves.toBeNull();
    await expect(pinFromClipboard()).resolves.toBeNull();
  });

  it('rejects an unexpected answer', async () => {
    mocks.invoke.mockResolvedValue({ kind: 'pong' });
    await expect(pinFromFile()).rejects.toEqual({
      code: 'internal',
      message: 'Snapline returned an unexpected Tools answer.',
    });
  });
});
