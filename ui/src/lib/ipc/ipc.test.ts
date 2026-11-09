import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));

import * as app from './app';
import * as history from './history';
import * as hotkeys from './hotkeys';
import * as record from './record';
import * as settings from './settings';
import * as tool from './tool';
import * as upload from './upload';
import * as workflow from './workflow';

describe('ipc pings', () => {
  beforeEach(() => {
    mocks.invoke.mockReset();
    mocks.invoke.mockResolvedValue({ kind: 'pong' });
  });

  const cases = [
    ['ipc_app', app.ping],
    ['ipc_settings', settings.ping],
    ['ipc_hotkeys', hotkeys.ping],
    ['ipc_history', history.ping],
    ['ipc_upload', upload.ping],
    ['ipc_workflow', workflow.ping],
    ['ipc_record', record.ping],
    ['ipc_tool', tool.ping],
  ] as const;

  it.each(cases)('%s round trips ping', async (command, ping) => {
    await expect(ping()).resolves.toEqual({ kind: 'pong' });
    expect(mocks.invoke).toHaveBeenCalledWith(command, { req: { kind: 'ping' } });
  });
});
