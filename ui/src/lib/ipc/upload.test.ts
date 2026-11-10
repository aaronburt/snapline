import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));

import {
  addDestination,
  cancelUpload,
  copyLink,
  exportSxcu,
  importSxcu,
  listDestinations,
  listUploadStates,
  pickAndUpload,
  removeDestination,
  saveDestination,
  testDestination,
} from './upload';

const unexpectedCopy = 'The link could not be copied.';
const unexpectedQueue = 'The files could not be added to the upload queue.';

beforeEach(() => {
  mocks.invoke.mockReset();
});

describe('upload IPC requests', () => {
  it('copies a link and rejects an unexpected acknowledgement', async () => {
    mocks.invoke.mockResolvedValueOnce({ kind: 'linkCopied' });

    await expect(copyLink('https://example.test/a')).resolves.toBeUndefined();
    expect(mocks.invoke).toHaveBeenLastCalledWith('ipc_upload', {
      req: { kind: 'copyLink', data: { link: 'https://example.test/a' } },
    });

    mocks.invoke.mockResolvedValueOnce({ kind: 'pong' });
    await expect(copyLink('https://example.test/a')).rejects.toThrow(unexpectedCopy);
  });

  it('returns all queued ids and rejects an unexpected reply', async () => {
    mocks.invoke.mockResolvedValueOnce({ kind: 'acceptedMany', data: { ids: [3, 8] } });

    await expect(pickAndUpload()).resolves.toEqual([3, 8]);
    expect(mocks.invoke).toHaveBeenLastCalledWith('ipc_upload', { req: { kind: 'pickAndUpload' } });

    mocks.invoke.mockResolvedValueOnce({ kind: 'cancelled' });
    await expect(pickAndUpload()).rejects.toThrow(unexpectedQueue);
  });

  it('returns upload states and treats an unrelated reply as an empty list', async () => {
    const states = [{ id: 11, status: 'uploading' }];
    mocks.invoke.mockResolvedValueOnce({ kind: 'states', data: states });
    await expect(listUploadStates()).resolves.toEqual(states);

    mocks.invoke.mockResolvedValueOnce({ kind: 'pong' });
    await expect(listUploadStates()).resolves.toEqual([]);
    expect(mocks.invoke).toHaveBeenLastCalledWith('ipc_upload', { req: { kind: 'list' } });
  });

  it('cancels an upload by its queue id', async () => {
    mocks.invoke.mockResolvedValueOnce({ kind: 'cancelled', data: { id: 12 } });
    await expect(cancelUpload(12)).resolves.toBeUndefined();
    expect(mocks.invoke).toHaveBeenCalledWith('ipc_upload', {
      req: { kind: 'cancel', data: { id: 12 } },
    });

    mocks.invoke.mockResolvedValueOnce({ kind: 'pong' });
    await expect(cancelUpload(12)).rejects.toThrow('The upload could not be cancelled.');
  });
});

describe('destination IPC requests', () => {
  const destinationView = { id: 'sftp-1', name: 'Home server' };
  const destinationResponse = {
    kind: 'destination',
    data: { kind: 'destination', data: destinationView },
  };

  it('lists destinations and uses an empty list for an unrelated reply', async () => {
    mocks.invoke.mockResolvedValueOnce({
      kind: 'destination',
      data: { kind: 'destinations', data: [destinationView] },
    });
    await expect(listDestinations()).resolves.toEqual([destinationView]);
    expect(mocks.invoke).toHaveBeenLastCalledWith('ipc_upload', {
      req: { kind: 'destination', data: { kind: 'list' } },
    });

    mocks.invoke.mockResolvedValueOnce({
      kind: 'destination',
      data: { kind: 'removed', data: { id: 'x' } },
    });
    await expect(listDestinations()).resolves.toEqual([]);
  });

  it.each([
    [
      'add',
      () => addDestination('sftp'),
      { kind: 'add', data: { kind: 'sftp' } },
      destinationResponse,
      destinationView,
    ],
    [
      'save',
      () => saveDestination({ kind: 'custom', name: 'Uploader', headers: [] }),
      { kind: 'save', data: { input: { kind: 'custom', name: 'Uploader', headers: [] } } },
      destinationResponse,
      destinationView,
    ],
    [
      'test',
      () => testDestination('sftp-1'),
      { kind: 'test', data: { id: 'sftp-1' } },
      {
        kind: 'destination',
        data: {
          kind: 'test',
          data: { outcome: { status: 'succeeded', link: 'https://example.test/1', message: null } },
        },
      },
      { status: 'succeeded', link: 'https://example.test/1', message: null },
    ],
    [
      'import',
      () => importSxcu(),
      { kind: 'importSxcu' },
      {
        kind: 'destination',
        data: {
          kind: 'imported',
          data: { destination: destinationView, warnings: ['review credentials'] },
        },
      },
      { destination: destinationView, warnings: ['review credentials'] },
    ],
    [
      'export without header values by default',
      () => exportSxcu('sftp-1'),
      { kind: 'exportSxcu', data: { id: 'sftp-1', includeHeaderValues: false } },
      { kind: 'destination', data: { kind: 'exported', data: { path: 'C:\\upload.sxcu' } } },
      'C:\\upload.sxcu',
    ],
    [
      'export with header values when requested',
      () => exportSxcu('sftp-1', true),
      { kind: 'exportSxcu', data: { id: 'sftp-1', includeHeaderValues: true } },
      { kind: 'destination', data: { kind: 'exported', data: { path: null } } },
      null,
    ],
  ] as const)(
    '%s forwards its payload and returns the result',
    async (_name, operation, request, response, expected) => {
      mocks.invoke.mockResolvedValue(response);

      await expect(operation()).resolves.toEqual(expected);
      expect(mocks.invoke).toHaveBeenCalledWith('ipc_upload', {
        req: { kind: 'destination', data: request },
      });
    },
  );

  it('removes a destination by id', async () => {
    mocks.invoke.mockResolvedValueOnce({
      kind: 'destination',
      data: { kind: 'removed', data: { id: 'sftp-1' } },
    });
    await expect(removeDestination('sftp-1')).resolves.toBeUndefined();
    expect(mocks.invoke).toHaveBeenCalledWith('ipc_upload', {
      req: { kind: 'destination', data: { kind: 'remove', data: { id: 'sftp-1' } } },
    });

    mocks.invoke.mockResolvedValueOnce({
      kind: 'destination',
      data: { kind: 'destination', data: destinationView },
    });
    await expect(removeDestination('sftp-1')).rejects.toThrow(
      'The destination could not be removed.',
    );
  });

  it.each([
    ['add', () => addDestination('sftp'), 'The destination could not be added.'],
    [
      'save',
      () => saveDestination({ kind: 'custom', headers: [] }),
      'The destination could not be saved.',
    ],
    ['test', () => testDestination('sftp-1'), 'The test upload could not be started.'],
    ['import', () => importSxcu(), 'The uploader could not be imported.'],
    ['export', () => exportSxcu('sftp-1'), 'The uploader could not be exported.'],
  ] as const)('%s rejects a response of the wrong kind', async (_name, operation, message) => {
    mocks.invoke.mockResolvedValue({
      kind: 'destination',
      data: { kind: 'removed', data: { id: 'sftp-1' } },
    });

    await expect(operation()).rejects.toThrow(message);
  });
});
