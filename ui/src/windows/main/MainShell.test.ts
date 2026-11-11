import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { flush } from '../../test/flush';
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

beforeEach(() => {
  currentRoute.set(resolve('#/workflows'));
  mocks.info.mockReset();
  mocks.call.mockReset();
  mocks.available.mockReset();
  mocks.info.mockResolvedValue({ kind: 'info', data: { hasMica: true } });
  mocks.call.mockResolvedValue({ kind: 'stats', data: { count: 4 } });
  mocks.available.mockResolvedValue(true);
});

afterEach(cleanup);

describe('MainShell accessibility', () => {
  it('has no axe violations with the workflows page, navigation and title bar', async () => {
    const { container } = render(MainShell);
    await flush();
    await waitFor(() => expect(mocks.call).toHaveBeenCalled());
    expect(screen.getByRole('navigation', { name: 'Main' })).not.toBeNull();
    expect(screen.getByRole('heading', { level: 1, name: 'Workflows' })).not.toBeNull();
    await expectAccessible(container);
  });
  it('searches settings from the sidebar and keeps the query when changing settings pages', async () => {
    currentRoute.set(resolve('#/settings/general'));
    render(MainShell);
    const search = screen.getByRole('searchbox', { name: 'Search settings' });
    expect(screen.getAllByRole('searchbox')).toHaveLength(1);
    await fireEvent.input(search, { target: { value: 'retry' } });
    expect(screen.queryByText('Retry failed uploads')).toBeNull();
    currentRoute.set(resolve('#/settings/uploads'));
    await flush();
    expect(
      (screen.getByRole('searchbox', { name: 'Search settings' }) as HTMLInputElement).value,
    ).toBe('retry');
    expect(screen.getByText('Retry failed uploads')).not.toBeNull();
  });
});
