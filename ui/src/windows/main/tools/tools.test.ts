import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render, screen } from '@testing-library/svelte';
import ToolsPage from './ToolsPage.svelte';

afterEach(cleanup);

describe('Tools page', () => {
  it('renders the eight cards in order', () => {
    const { container } = render(ToolsPage);
    const cards = Array.from(container.querySelectorAll('.grid button'));
    expect(cards.map((card) => card.querySelector('.name')?.textContent)).toEqual([
      'Colour picker',
      'Text capture (OCR)',
      'Screen ruler',
      'QR code',
      'Image editor',
      'Pin to screen',
      'Image combiner',
      'Video converter',
    ]);
  });

  it('disables every card until milestone 9', () => {
    const { container } = render(ToolsPage);
    const cards = Array.from(container.querySelectorAll<HTMLButtonElement>('.grid button'));
    expect(cards.length).toBe(8);
    expect(cards.every((card) => card.disabled)).toBe(true);
    expect(screen.getAllByText('Coming in a later version').length).toBe(8);
  });

  it('keeps the default chords visible', () => {
    render(ToolsPage);
    expect(screen.getByText('Ctrl+Shift+C')).not.toBeNull();
    expect(screen.getByText('Ctrl+Shift+T')).not.toBeNull();
    expect(screen.getByText('Ctrl+Shift+M')).not.toBeNull();
    expect(screen.getAllByText('—').length).toBe(5);
  });
});
