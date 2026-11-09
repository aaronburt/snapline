import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import { expectAccessible } from '../../test/axe';
import Nav from './Nav.svelte';
import { settingsDto } from './settings/settingsDto';

const mocks = vi.hoisted(() => ({
  call: vi.fn(),
}));

vi.mock('../../lib/ipc/core', () => ({ call: mocks.call }));

async function flush(): Promise<void> {
  await tick();
  await Promise.resolve();
  await tick();
}

beforeEach(() => {
  mocks.call.mockReset();
  mocks.call.mockResolvedValue({ kind: 'stats', data: { count: 12 } });
});

afterEach(cleanup);

describe('Nav', () => {
  it('shows the real capture count from the stats request', async () => {
    const { container } = render(Nav, { props: { page: 'history' } });
    await flush();
    await expectAccessible(container);
    expect(mocks.call).toHaveBeenCalledWith('ipc_app', { kind: 'stats' });
    expect(container.querySelector('.count')?.textContent).toBe('12');
    expect(screen.getByRole('link', { name: /History/ })).not.toBeNull();
  });

  it('shows zero when there are no captures', async () => {
    mocks.call.mockResolvedValue({ kind: 'stats', data: { count: 0 } });
    const { container } = render(Nav, { props: { page: 'history' } });
    await flush();
    expect(container.querySelector('.count')?.textContent).toBe('0');
  });

  it('shows the configured image destination instead of a fixed local-only label', async () => {
    const dto = settingsDto();
    dto.uploads.images = 'my-uploader';
    mocks.call.mockImplementation(async (area: string) => {
      if (area === 'ipc_settings') return { kind: 'settings', data: { settings: dto } };
      if (area === 'ipc_upload') return { kind: 'destination', data: { kind: 'destinations', data: [{ id: 'my-uploader', name: 'My uploader' }] } };
      return { kind: 'stats', data: { count: 0 } };
    });
    render(Nav, { props: { page: 'history' } });
    await flush();
    expect(screen.getByRole('link', { name: /Default destination: My uploader/ })).not.toBeNull();
    expect(screen.getByRole('link', { name: 'Tools' })).not.toBeNull();
  });

  it('refreshes the count when the main window regains focus', async () => {
    render(Nav, { props: { page: 'history' } });
    await flush();
    expect(mocks.call.mock.calls.filter(([area]) => area === 'ipc_app')).toHaveLength(1);
    mocks.call.mockResolvedValue({ kind: 'stats', data: { count: 13 } });
    window.dispatchEvent(new Event('focus'));
    await flush();
    expect(mocks.call.mock.calls.filter(([area]) => area === 'ipc_app')).toHaveLength(2);
    expect(screen.getByText('13')).not.toBeNull();
  });
});
