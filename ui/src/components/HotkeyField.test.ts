import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import HotkeyField from './HotkeyField.svelte';

afterEach(cleanup);

describe('HotkeyField', () => {
  it('captures a chord and reports it', async () => {
    const onchange = vi.fn();
    render(HotkeyField, {
      props: { label: 'Capture region', value: 'PrtSc', onchange },
    });
    await fireEvent.click(screen.getByRole('button', { name: 'Change' }));
    expect((screen.getByLabelText('Capture region') as HTMLInputElement).value).toBe(
      'Press keys…',
    );
    await fireEvent.keyDown(window, { key: 'g', ctrlKey: true, shiftKey: true });
    expect(onchange).toHaveBeenCalledWith('Ctrl+Shift+G');
    expect((screen.getByLabelText('Capture region') as HTMLInputElement).value).toBe(
      'Ctrl+Shift+G',
    );
  });

  it('cancels with Escape and clears with Backspace', async () => {
    render(HotkeyField, { props: { label: 'Record GIF', value: 'Ctrl+Shift+G' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Change' }));
    await fireEvent.keyDown(window, { key: 'Escape' });
    expect((screen.getByLabelText('Record GIF') as HTMLInputElement).value).toBe(
      'Ctrl+Shift+G',
    );
    await fireEvent.click(screen.getByRole('button', { name: 'Change' }));
    await fireEvent.keyDown(window, { key: 'Backspace' });
    expect((screen.getByLabelText('Record GIF') as HTMLInputElement).value).toBe('');
  });

  it('shows the Windows reserved message and clash text', async () => {
    render(HotkeyField, {
      props: { label: 'Capture window', value: 'Alt+PrtSc', clash: 'Ctrl+Alt+S is already used' },
    });
    expect(screen.getByText('Ctrl+Alt+S is already used')).not.toBeNull();
    await fireEvent.click(screen.getByRole('button', { name: 'Change' }));
    await fireEvent.keyDown(window, { key: 'l', metaKey: true });
    expect(screen.getByText('Windows uses this shortcut.')).not.toBeNull();
  });
});
