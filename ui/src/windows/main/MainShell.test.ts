import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { expectAccessible } from '../../test/axe';
import { currentRoute, resolve } from '../../router';
import MainShell from './MainShell.svelte';

const mocks = vi.hoisted(() => ({
  info: vi.fn(),
  call: vi.fn(),
  available: vi.fn(),
}));

const appWindow = vi.hoisted(() => ({
  minimize: vi.fn(),
  toggleMaximize: vi.fn(),
  close: vi.fn(),
  isMaximized: vi.fn().mockResolvedValue(false),
  onResized: vi.fn().mockResolvedValue(() => undefined),
}));

vi.mock('../../lib/ipc/app', () => ({
  info: mocks.info,
  ready: vi.fn().mockResolvedValue(undefined),
}));
vi.mock('../../lib/ipc/core', () => ({ call: mocks.call }));
vi.mock('../../lib/ipc/tool', () => ({
  toolAvailable: mocks.available,
  editorOpenFile: vi.fn(),
  pinFromFile: vi.fn(),
  pinFromClipboard: vi.fn(),
}));
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => appWindow,
}));

async function flush(): Promise<void> {
  await tick();
  await Promise.resolve();
  await tick();
  await Promise.resolve();
  await tick();
}

beforeEach(() => {
  currentRoute.set(resolve('#/tools'));
  mocks.info.mockReset();
  mocks.call.mockReset();
  mocks.available.mockReset();
  mocks.info.mockResolvedValue({ kind: 'info', data: { hasMica: true } });
  mocks.call.mockResolvedValue({ kind: 'stats', data: { count: 4 } });
  mocks.available.mockResolvedValue(true);
});

afterEach(cleanup);

describe('MainShell accessibility', () => {
  it('has no axe violations with the tools page, navigation and title bar', async () => {
    const { container } = render(MainShell);
    await flush();
    await waitFor(() => expect(mocks.call).toHaveBeenCalled());
    expect(screen.getByRole('navigation', { name: 'Main' })).not.toBeNull();
    expect(screen.getByRole('heading', { level: 1, name: 'Tools' })).not.toBeNull();
    await expectAccessible(container);
  });
});
