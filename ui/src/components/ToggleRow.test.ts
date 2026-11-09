import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import ToggleRow from './ToggleRow.svelte';

afterEach(cleanup);

describe('ToggleRow', () => {
  it('shows the title, description and On state', () => {
    render(ToggleRow, {
      props: {
        icon: 'bell',
        title: 'Notifications',
        description: 'Show a toast when an upload finishes or a link is copied',
        checked: true,
      },
    });
    expect(screen.getByText('Notifications')).not.toBeNull();
    expect(
      screen.getByText('Show a toast when an upload finishes or a link is copied'),
    ).not.toBeNull();
    expect(screen.getByText('On')).not.toBeNull();
    expect((screen.getByRole('checkbox') as HTMLInputElement).checked).toBe(true);
  });

  it('reports a change when toggled', async () => {
    const onchange = vi.fn();
    render(ToggleRow, { props: { icon: 'bell', title: 'Notifications', onchange } });
    const checkbox = screen.getByRole('checkbox');
    await fireEvent.click(checkbox);
    expect(onchange).toHaveBeenCalledWith(true);
  });
});
