import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render, screen } from '@testing-library/svelte';
import NavItem from './NavItem.svelte';

afterEach(cleanup);

describe('NavItem', () => {
  it('shows the label, count and current page marker', () => {
    render(NavItem, { props: { label: 'History', icon: 'history', count: '248', active: true } });
    const item = screen.getByRole('button', { name: /History/ });
    expect(item.getAttribute('aria-current')).toBe('page');
    expect(screen.getByText('248')).not.toBeNull();
    expect(screen.getByText('History')).not.toBeNull();
  });

  it('renders a link when given a href', () => {
    render(NavItem, {
      props: { label: 'Settings', icon: 'gear', href: '#/settings/general' },
    });
    expect(screen.getByRole('link', { name: 'Settings' })).not.toBeNull();
  });
});
