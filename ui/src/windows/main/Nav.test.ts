import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { get } from 'svelte/store';
import { flush } from '../../test/flush';
import { expectAccessible } from '../../test/axe';
import Nav from './Nav.svelte';
import { searchQuery } from './history/search';
import { currentRoute, resolve } from '../../router';

const mocks = vi.hoisted(() => ({
  call: vi.fn(),
}));

vi.mock('../../lib/ipc/core', () => ({ call: mocks.call }));

beforeEach(() => {
  mocks.call.mockReset();
  mocks.call.mockResolvedValue({ kind: 'stats', data: { count: 12 } });
});

afterEach(cleanup);

describe('Nav', () => {
  it('shows a failed capture as a dismissible toast', async () => {
    mocks.call.mockImplementation(async (_area: string, req: { kind: string }) => {
      if (req.kind === 'panelAction') throw { message: 'Capture is unavailable.' };
      return { kind: 'stats', data: { count: 1 } };
    });
    render(Nav, { props: { page: 'history' } });
    await flush();
    await fireEvent.click(screen.getByRole('button', { name: /New capture/ }));
    await flush();
    const toast = screen.getByRole('alert');
    expect(toast.textContent).toContain('Capture is unavailable.');
    expect(toast.closest('nav')).toBeNull();
    await fireEvent.click(screen.getByRole('button', { name: 'Dismiss' }));
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('searches the history from the sidebar', async () => {
    vi.useFakeTimers();
    window.location.hash = '#/workflows';
    currentRoute.set(resolve('#/workflows'));
    searchQuery.set('');
    render(Nav, { props: { page: 'workflows' } });
    try {
      const input = screen.getByRole('searchbox', { name: 'Search captures' });
      await fireEvent.input(input, { target: { value: 'invoice' } });
      expect(get(searchQuery)).toBe('');
      vi.advanceTimersByTime(250);
      expect(get(searchQuery)).toBe('invoice');
      expect(window.location.hash).toBe('#/history');
    } finally {
      vi.useRealTimers();
    }
  });

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

  it('lists the sections and the settings pages in the sidebar', async () => {
    render(Nav, { props: { page: 'history' } });
    await flush();
    expect(screen.getByRole('link', { name: 'Workflows' })).not.toBeNull();
    expect(screen.getByRole('heading', { name: 'Settings' })).not.toBeNull();
    for (const name of ['General', 'Capture', 'Hotkeys', 'Upload destinations']) {
      expect(screen.getByRole('link', { name })).not.toBeNull();
    }
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
