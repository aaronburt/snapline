import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen } from '@testing-library/svelte';
import TitleBar from './TitleBar.svelte';

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

describe('TitleBar', () => {
  it('shows the product name and the search field', () => {
    render(TitleBar);
    expect(screen.getByText('Snapline')).not.toBeNull();
    expect(screen.getByLabelText('Search captures')).not.toBeNull();
    expect(
      screen.getByPlaceholderText('Search by name, app, text in image…'),
    ).not.toBeNull();
    expect(screen.getByRole('button', { name: 'Minimise' })).not.toBeNull();
    expect(screen.getByRole('button', { name: 'Maximise' })).not.toBeNull();
    expect(screen.getByRole('button', { name: 'Close' })).not.toBeNull();
  });
});
