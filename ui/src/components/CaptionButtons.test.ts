import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import CaptionButtons from './CaptionButtons.svelte';

const appWindow = vi.hoisted(() => ({
  minimize: vi.fn(),
  toggleMaximize: vi.fn(),
  close: vi.fn(),
  isMaximized: vi.fn().mockResolvedValue(false),
  onResized: vi.fn().mockResolvedValue(() => undefined),
}));

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => appWindow,
}));

afterEach(cleanup);

describe('CaptionButtons', () => {
  it('calls the window commands', async () => {
    render(CaptionButtons);
    await fireEvent.click(screen.getByRole('button', { name: 'Minimise' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Maximise' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Close' }));
    expect(appWindow.minimize).toHaveBeenCalledOnce();
    expect(appWindow.toggleMaximize).toHaveBeenCalledOnce();
    expect(appWindow.close).toHaveBeenCalledOnce();
  });
});
