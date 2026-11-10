import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));

import {
  historyConfirmDelete,
  historyCopyImage,
  historyDelete,
  historyGet,
  historyOpenFile,
  historyOpenUrl,
  historyPage,
  historySearch,
  historyShowInFolder,
  historyStats,
} from './history';

const page = { items: [], next: { createdAt: 12, id: 7 } };
const unexpected = {
  code: 'internal',
  message: 'Snapline returned an unexpected History answer.',
};

beforeEach(() => {
  mocks.invoke.mockReset();
});

describe('history IPC requests', () => {
  it.each([
    [
      'page',
      () => historyPage('images', { createdAt: 20, id: 3 }),
      { kind: 'page', data: { filter: 'images', cursor: { createdAt: 20, id: 3 } } },
      { kind: 'page', data: { page } },
    ],
    [
      'search',
      () => historySearch('screenshot', 'uploaded', null),
      { kind: 'search', data: { text: 'screenshot', filter: 'uploaded', cursor: null } },
      { kind: 'search', data: { page } },
    ],
  ] as const)('%s preserves its filter and cursor', async (_name, request, expectedRequest, response) => {
    mocks.invoke.mockResolvedValue(response);

    await expect(request()).resolves.toEqual(page);
    expect(mocks.invoke).toHaveBeenCalledWith('ipc_history', { req: expectedRequest });
  });

  it.each([
    [
      'get',
      () => historyGet(7),
      { kind: 'get', data: { id: 7 } },
      { kind: 'get', data: { capture: { id: 7, name: 'screen.png' } } },
      { id: 7, name: 'screen.png' },
    ],
    [
      'stats',
      () => historyStats(),
      { kind: 'stats' },
      { kind: 'stats', data: { stats: { count: 4, bytes: 10, uploadedThisWeek: 2 } } },
      { count: 4, bytes: 10, uploadedThisWeek: 2 },
    ],
    [
      'confirm delete',
      () => historyConfirmDelete(7),
      { kind: 'confirmDelete', data: { id: 7 } },
      { kind: 'confirmDelete', data: { confirmed: false } },
      false,
    ],
  ] as const)('%s sends its request and returns the payload', async (_name, request, expectedRequest, response, expected) => {
    mocks.invoke.mockResolvedValue(response);

    await expect(request()).resolves.toEqual(expected);
    expect(mocks.invoke).toHaveBeenCalledWith('ipc_history', { req: expectedRequest });
  });

  it.each([
    ['delete', historyDelete, { kind: 'delete', data: { id: 7 } }],
    ['copy image', historyCopyImage, { kind: 'copyImage', data: { id: 7 } }],
    ['show in folder', historyShowInFolder, { kind: 'showInFolder', data: { id: 7 } }],
    ['open file', historyOpenFile, { kind: 'openFile', data: { id: 7 } }],
    ['open URL', historyOpenUrl, { kind: 'openUrl', data: { id: 7 } }],
  ] as const)('%s sends the capture id and accepts its acknowledgement', async (_name, request, expectedRequest) => {
    mocks.invoke.mockResolvedValue({ kind: expectedRequest.kind });

    await expect(request(7)).resolves.toBeUndefined();
    expect(mocks.invoke).toHaveBeenCalledWith('ipc_history', {
      req: expectedRequest,
    });
  });
});

describe('history IPC response validation', () => {
  it.each([
    ['page', () => historyPage('all', null)],
    ['search', () => historySearch('term', 'all', null)],
    ['get', () => historyGet(7)],
    ['stats', () => historyStats()],
    ['confirm delete', () => historyConfirmDelete(7)],
    ['delete', () => historyDelete(7)],
    ['copy image', () => historyCopyImage(7)],
    ['show in folder', () => historyShowInFolder(7)],
    ['open file', () => historyOpenFile(7)],
    ['open URL', () => historyOpenUrl(7)],
  ] as const)('%s reports a mismatched response', async (_name, request) => {
    mocks.invoke.mockResolvedValue({ kind: 'pong' });

    await expect(request()).rejects.toEqual(unexpected);
  });
});
