import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));

import {
  convertCancel,
  convertPickFile,
  convertProgress,
  convertResult,
  convertStart,
  editorOpenClipboard,
  editorOpenFile,
  editorOpenHistory,
  pinFromClipboard,
  pinFromFile,
  pinHistory,
  pinMenu,
  qrCopyImage,
  qrCopyText,
  qrMake,
  qrOpen,
  qrSavePng,
  qrScan,
  toolAvailable,
} from './tool';

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

  it('uses the pin menu and pins history captures', async () => {
    mocks.invoke.mockResolvedValueOnce({ kind: 'pinned', data: { source: 'clipboard' } });
    await expect(pinMenu()).resolves.toBe('clipboard');
    expect(mocks.invoke).toHaveBeenCalledWith('ipc_tool', { req: { kind: 'pinMenu' } });

    mocks.invoke.mockResolvedValueOnce({ kind: 'pinned', data: { source: 'file' } });
    await expect(pinHistory(24)).resolves.toBeUndefined();
    expect(mocks.invoke).toHaveBeenLastCalledWith('ipc_tool', {
      req: { kind: 'pinHistory', data: { captureId: 24 } },
    });
  });

  it('reports a failed history pin instead of treating it as cancelled', async () => {
    mocks.invoke.mockResolvedValue({ kind: 'pinCancelled' });

    await expect(pinHistory(24)).rejects.toThrow('The capture could not be pinned.');
  });
});

describe('tool IPC requests', () => {
  const outcome = { action: null, captureId: 8, path: 'C:\\capture.png', cancelled: false };
  const progress = { phase: 'converting', percent: 40, outTimeMs: 300, speed: 1.2, message: null };
  const converted = { captureId: 31, path: 'C:\\converted.gif', name: 'converted.gif', format: 'gif' };

  it.each([
    [
      'availability',
      () => toolAvailable('qr'),
      { kind: 'available', data: { tool: 'qr' } },
      { kind: 'available', data: { available: false } },
      false,
    ],
    [
      'open a History capture in the editor',
      () => editorOpenHistory(8),
      { kind: 'editorOpenHistory', data: { captureId: 8 } },
      { kind: 'editorOpened', data: { outcome } },
      outcome,
    ],
    ['open a file in the editor', editorOpenFile, { kind: 'editorOpenFile' }, { kind: 'editorOpened', data: { outcome } }, outcome],
    [
      'open the clipboard in the editor',
      editorOpenClipboard,
      { kind: 'editorOpenClipboard' },
      { kind: 'editorOpened', data: { outcome } },
      outcome,
    ],
    [
      'make a QR code',
      () => qrMake('hello'),
      { kind: 'qrMake', data: { text: 'hello' } },
      { kind: 'qrMade', data: { svg: '<svg/>', link: null } },
      { svg: '<svg/>', link: null },
    ],
    [
      'scan the current capture',
      qrScan,
      { kind: 'qrScan', data: { captureId: null } },
      { kind: 'qrScanned', data: { text: 'https://example.test', link: 'https://example.test', isUrl: true } },
      { text: 'https://example.test', link: 'https://example.test', isUrl: true },
    ],
    [
      'copy QR text',
      () => qrCopyText('hello'),
      { kind: 'qrCopyText', data: { text: 'hello' } },
      { kind: 'qrCopied' },
      undefined,
    ],
    [
      'copy QR image',
      () => qrCopyImage('hello'),
      { kind: 'qrCopyImage', data: { text: 'hello' } },
      { kind: 'qrCopied' },
      undefined,
    ],
    [
      'save QR image',
      () => qrSavePng('hello'),
      { kind: 'qrSavePng', data: { text: 'hello' } },
      { kind: 'qrSaved', data: { path: 'C:\\qr.png' } },
      'C:\\qr.png',
    ],
    [
      'open QR link',
      () => qrOpen('https://example.test'),
      { kind: 'qrOpen', data: { link: 'https://example.test' } },
      { kind: 'qrOpened' },
      undefined,
    ],
    [
      'pick a conversion source',
      convertPickFile,
      { kind: 'convertPickFile' },
      { kind: 'convertFilePicked', data: { path: null } },
      null,
    ],
    [
      'start a conversion',
      () => convertStart({ kind: 'capture', data: { captureId: 8 } }, 'webmVp9'),
      { kind: 'convertStart', data: { source: { kind: 'capture', data: { captureId: 8 } }, format: 'webmVp9' } },
      { kind: 'convertStarted', data: { jobId: 50 } },
      50,
    ],
    [
      'read conversion progress',
      () => convertProgress(50),
      { kind: 'convertProgress', data: { jobId: 50 } },
      { kind: 'convertProgressed', data: { jobId: 50, progress } },
      progress,
    ],
    [
      'cancel a conversion',
      () => convertCancel(50),
      { kind: 'convertCancel', data: { jobId: 50 } },
      { kind: 'convertCancelled', data: { jobId: 50 } },
      undefined,
    ],
    [
      'read a conversion result',
      () => convertResult(50),
      { kind: 'convertResult', data: { jobId: 50 } },
      { kind: 'converted', data: { converted } },
      converted,
    ],
  ] as const)('%s sends the request and returns its response data', async (_name, operation, request, response, expected) => {
    mocks.invoke.mockResolvedValue(response);

    await expect(operation()).resolves.toEqual(expected);
    expect(mocks.invoke).toHaveBeenCalledWith('ipc_tool', { req: request });
  });

  it.each([
    ['availability', () => toolAvailable('qr')],
    ['editor launch', editorOpenFile],
    ['QR creation', () => qrMake('hello')],
    ['conversion progress', () => convertProgress(50)],
  ] as const)('%s rejects a reply from a different tool action', async (_name, operation) => {
    mocks.invoke.mockResolvedValue({ kind: 'pong' });

    await expect(operation()).rejects.toEqual({
      code: 'internal',
      message: 'Snapline returned an unexpected Tools answer.',
    });
  });
});
