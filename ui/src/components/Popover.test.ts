import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
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

  it('traps Tab inside the dialog and returns focus on close', async () => {
    const children = createRawSnippet(() => ({
      render: () =>
        '<div><button type="button">First</button><button type="button">Last</button></div>',
    }));
    const trigger = document.createElement('button');
    trigger.textContent = 'Open';
    document.body.append(trigger);
    trigger.focus();
    const { unmount } = render(Popover, {
      props: { open: true, label: 'QR code', children },
    });
    await Promise.resolve();
    const first = screen.getByRole('button', { name: 'First' });
    const last = screen.getByRole('button', { name: 'Last' });
    await waitFor(() => expect(document.activeElement).toBe(first));
    last.focus();
    await fireEvent.keyDown(window, { key: 'Tab' });
    expect(document.activeElement).toBe(first);
    first.focus();
    await fireEvent.keyDown(window, { key: 'Tab', shiftKey: true });
    expect(document.activeElement).toBe(last);
    unmount();
    expect(document.activeElement).toBe(trigger);
    trigger.remove();
  });
});
