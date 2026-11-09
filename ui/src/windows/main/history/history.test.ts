import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import HistoryPage from './HistoryPage.svelte';
import { searchQuery } from './search';
import {
  captures,
  filterCaptures,
  groupCaptures,
  moveSelection,
} from './data';

afterEach(() => {
  cleanup();
  searchQuery.set('');
});

describe('History filters', () => {
  it('filters by type and status', () => {
    expect(filterCaptures(captures, 'All', '').length).toBe(10);
    expect(filterCaptures(captures, 'Images', '').length).toBe(7);
    expect(filterCaptures(captures, 'Videos', '').map((item) => item.id)).toEqual([2, 9]);
    expect(filterCaptures(captures, 'GIFs', '').map((item) => item.id)).toEqual([5]);
    expect(filterCaptures(captures, 'Uploaded', '').length).toBe(7);
    expect(filterCaptures(captures, 'Local only', '').map((item) => item.id)).toEqual([3, 9]);
  });

  it('filters by search text across name and app', () => {
    expect(filterCaptures(captures, 'All', 'recording').map((item) => item.id)).toEqual([2, 9]);
    expect(filterCaptures(captures, 'All', 'chrome.exe').map((item) => item.id)).toEqual([1, 8]);
    expect(filterCaptures(captures, 'All', 'nothing here')).toEqual([]);
  });

  it('groups captures and counts them', () => {
    const groups = groupCaptures(captures);
    expect(groups.map((group) => group.label)).toEqual(['Today', 'Yesterday']);
    expect(groups[0]?.count).toBe('6 captures');
    expect(groups[1]?.count).toBe('4 captures');
  });

  it('moves the selection and clamps at the ends', () => {
    expect(moveSelection(captures, 1, 1)).toBe(2);
    expect(moveSelection(captures, 3, -1)).toBe(2);
    expect(moveSelection(captures, 10, 1)).toBe(10);
    expect(moveSelection(captures, 1, -1)).toBe(1);
    expect(moveSelection([], 1, 1)).toBeNull();
  });
});

describe('History page', () => {
  it('shows the board groups and the first capture detail', () => {
    const { container } = render(HistoryPage);
    expect(screen.getByRole('heading', { level: 1, name: 'History' })).not.toBeNull();
    expect(screen.getByRole('heading', { level: 2, name: /Today/ })).not.toBeNull();
    expect(screen.getByRole('heading', { level: 2, name: /Yesterday/ })).not.toBeNull();
    expect(container.querySelectorAll('section[aria-label="Today"] button').length).toBe(6);
    expect(container.querySelector('aside')?.textContent).toContain(
      'Region_2026-11-08_14-31-07.png',
    );
  });

  it('switches the segmented filter and clears the selection', async () => {
    const { container } = render(HistoryPage);
    await fireEvent.click(screen.getByRole('radio', { name: 'Videos' }));
    expect(container.querySelectorAll('section[aria-label="Today"] button').length).toBe(1);
    expect(container.querySelector('aside')?.textContent).toContain(
      'Select a capture to see its details.',
    );
  });

  it('toggles between grid and list views', async () => {
    const { container } = render(HistoryPage);
    const list = screen.getByRole('button', { name: 'List view' });
    expect(list.getAttribute('aria-pressed')).toBe('false');
    await fireEvent.click(list);
    expect(list.getAttribute('aria-pressed')).toBe('true');
    expect(container.querySelector('table')).not.toBeNull();
    expect(screen.getByRole('columnheader', { name: 'Captured' })).not.toBeNull();
    const grid = screen.getByRole('button', { name: 'Grid view' });
    await fireEvent.click(grid);
    expect(container.querySelector('table')).toBeNull();
  });

  it('shows the empty state when nothing matches', async () => {
    render(HistoryPage);
    searchQuery.set('no such capture');
    await tick();
    expect(screen.getByText('No captures yet')).not.toBeNull();
    expect(screen.getByText('Press PrtSc to capture a region.')).not.toBeNull();
  });

  it('selects a capture from the list view', async () => {
    const { container } = render(HistoryPage);
    await fireEvent.click(screen.getByRole('button', { name: 'List view' }));
    const rows = container.querySelectorAll('tbody tr');
    await fireEvent.click(rows[2] as Element);
    expect(container.querySelector('aside')?.textContent).toContain(
      'Window_2026-11-08_13-40-11.png',
    );
  });
});
