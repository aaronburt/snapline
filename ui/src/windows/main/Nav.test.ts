import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import Nav from './Nav.svelte';

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

  it('refreshes the count when the main window regains focus', async () => {
    render(Nav, { props: { page: 'history' } });
    await flush();
    expect(mocks.call).toHaveBeenCalledTimes(1);
    mocks.call.mockResolvedValue({ kind: 'stats', data: { count: 13 } });
    window.dispatchEvent(new Event('focus'));
    await flush();
    expect(mocks.call).toHaveBeenCalledTimes(2);
    expect(screen.getByText('13')).not.toBeNull();
  });
});
