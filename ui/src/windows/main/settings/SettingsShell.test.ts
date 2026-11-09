import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import SettingsShell from './SettingsShell.svelte';
import { currentRoute, resolve } from '../../../router';
import { resetSettings } from './settings';

beforeEach(() => {
  resetSettings();
  currentRoute.set(resolve('#/settings/general'));
});

afterEach(cleanup);

describe('SettingsShell', () => {
  it('shows the settings navigation', () => {
    render(SettingsShell);
    expect(screen.getByRole('link', { name: 'Back to Snapline' })).not.toBeNull();
    expect(screen.getByPlaceholderText('Find a setting')).not.toBeNull();
    expect(screen.getByRole('link', { name: 'General' })).not.toBeNull();
    expect(screen.getByRole('link', { name: 'Capture' })).not.toBeNull();
    expect(screen.getByRole('link', { name: 'Hotkeys' })).not.toBeNull();
    expect(screen.getByRole('link', { name: 'Upload destinations' })).not.toBeNull();
  });

  it('filters the current page and lists matches on other pages', async () => {
    render(SettingsShell);
    const search = screen.getByPlaceholderText('Find a setting');
    await fireEvent.input(search, { target: { value: 'launch' } });
    expect(screen.getByText('Launch at sign-in')).not.toBeNull();
    expect(screen.queryByText('On other pages')).toBeNull();
    await fireEvent.input(search, { target: { value: 'retry' } });
    expect(screen.getByText('On other pages')).not.toBeNull();
    expect(screen.getByRole('link', { name: /Retry failed uploads/ })).not.toBeNull();
    expect(screen.queryByText('Launch at sign-in')).toBeNull();
  });
});
