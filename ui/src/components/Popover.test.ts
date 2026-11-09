import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { createRawSnippet } from 'svelte';
import Popover from './Popover.svelte';

afterEach(cleanup);

describe('Popover', () => {
  it('renders its content and closes on Escape', async () => {
    const onclose = vi.fn();
    const children = createRawSnippet(() => ({ render: () => '<span>QR code</span>' }));
    render(Popover, { props: { open: true, label: 'QR code', children, onclose } });
    expect(screen.getByRole('dialog', { name: 'QR code' })).not.toBeNull();
    expect(screen.getByText('QR code')).not.toBeNull();
    await fireEvent.keyDown(window, { key: 'Escape' });
    expect(onclose).toHaveBeenCalledOnce();
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('closes on an outside pointer down', async () => {
    const onclose = vi.fn();
    render(Popover, { props: { open: true, label: 'QR code', onclose } });
    await fireEvent.pointerDown(document.body);
    expect(onclose).toHaveBeenCalledOnce();
  });
});
