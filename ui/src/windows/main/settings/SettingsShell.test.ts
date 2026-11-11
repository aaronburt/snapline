import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, render, screen } from '@testing-library/svelte';
import { expectAccessible } from '../../../test/axe';
import SettingsShell from './SettingsShell.svelte';
import { currentRoute, resolve } from '../../../router';
import { resetSettings } from './settings';

beforeEach(() => {
  resetSettings();
  currentRoute.set(resolve('#/settings/general'));
});

afterEach(cleanup);

describe('SettingsShell', () => {
  it('filters the current page without listing matches on other pages', async () => {
    const { container, rerender } = render(SettingsShell, { query: 'launch' });
    await expectAccessible(container);
    expect(screen.getByText('Launch at sign-in')).not.toBeNull();
    expect(screen.queryByText('On other pages')).toBeNull();
    await rerender({ query: 'retry' });
    expect(screen.queryByText('On other pages')).toBeNull();
    expect(screen.queryByRole('link', { name: /Retry failed uploads/ })).toBeNull();
    expect(screen.queryByText('Launch at sign-in')).toBeNull();
  });
});
