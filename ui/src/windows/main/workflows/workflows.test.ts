import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import WorkflowsPage from './WorkflowsPage.svelte';
import { setWorkflowEnabled, workflows } from './data';

afterEach(cleanup);

describe('Workflows page', () => {
  it('renders the five design workflows', () => {
    render(WorkflowsPage);
    expect(screen.getByText('Screenshot to link (default)')).not.toBeNull();
    expect(screen.getByText('Annotate and share')).not.toBeNull();
    expect(screen.getByText('Bug report clip')).not.toBeNull();
    expect(screen.getByText('Grab text')).not.toBeNull();
    expect(screen.getByText('Save quietly')).not.toBeNull();
  });

  it('disables the default toggle and explains why', () => {
    render(WorkflowsPage);
    const toggle = screen.getByRole('checkbox', {
      name: 'Enable Screenshot to link (default)',
    }) as HTMLInputElement;
    expect(toggle.disabled).toBe(true);
    expect(toggle.checked).toBe(true);
    expect(toggle.getAttribute('title')).toBe(
      'The default workflow runs on PrtSc and from the tray',
    );
    expect(screen.getByText('PrtSc')).not.toBeNull();
  });

  it('keeps the default workflow enabled when its toggle is clicked', async () => {
    render(WorkflowsPage);
    const toggle = screen.getByRole('checkbox', {
      name: 'Enable Screenshot to link (default)',
    }) as HTMLInputElement;
    await fireEvent.click(toggle);
    expect(toggle.checked).toBe(true);
  });

  it('toggles a normal workflow on', async () => {
    render(WorkflowsPage);
    const toggle = screen.getByRole('checkbox', { name: 'Enable Grab text' }) as HTMLInputElement;
    expect(toggle.checked).toBe(false);
    await fireEvent.click(toggle);
    expect(toggle.checked).toBe(true);
  });
});

describe('setWorkflowEnabled', () => {
  it('guards the default workflow', () => {
    const updated = setWorkflowEnabled(workflows, 'f1', false);
    expect(updated.find((workflow) => workflow.id === 'f1')?.enabled).toBe(true);
  });

  it('updates a normal workflow', () => {
    const updated = setWorkflowEnabled(workflows, 'f4', true);
    expect(updated.find((workflow) => workflow.id === 'f4')?.enabled).toBe(true);
  });
});
