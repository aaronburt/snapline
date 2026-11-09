import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { expectAccessible } from '../../../test/axe';

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));

import type { PreviewDto } from '../../../lib/ipc/bindings/PreviewDto';
import type { WorkflowInfo } from '../../../lib/ipc/bindings/WorkflowInfo';
import type { WorkflowInput } from '../../../lib/ipc/bindings/WorkflowInput';
import { currentRoute } from '../../../router';
import WorkflowEditor from './WorkflowEditor.svelte';

interface Request {
  kind: string;
  data?: {
    id?: string;
    workflow?: WorkflowInput;
    owner?: string;
    chord?: string;
  };
}

interface Options {
  workflows?: WorkflowInfo[];
  workflow?: WorkflowInfo;
  preview?: PreviewDto;
}

const defaultWorkflow: WorkflowInfo = {
  id: 'default',
  name: 'Screenshot to link',
  hotkey: null,
  enabled: true,
  steps: [{ step: 'copy_image' }, { step: 'save' }],
  isDefault: true,
};

const annotate: WorkflowInfo = {
  id: 'annotate-and-share',
  name: 'Annotate and share',
  hotkey: 'Ctrl+Shift+A',
  enabled: true,
  steps: [{ step: 'capture', kind: 'region' }, { step: 'annotate' }],
  isDefault: false,
};

function install(options: Options = {}): void {
  const workflows = options.workflows ?? [defaultWorkflow, annotate];
  mocks.invoke.mockImplementation((command: string, args: { req: Request }) => {
    const req = args.req;
    if (command === 'ipc_hotkeys' && req.kind === 'get') {
      return Promise.resolve({
        kind: 'state',
        data: { bindings: [], blocked: [], paused: false },
      });
    }
    if (command === 'ipc_hotkeys' && req.kind === 'preview') {
      return Promise.resolve({
        kind: 'preview',
        data: { preview: options.preview ?? 'free' },
      });
    }
    if (command !== 'ipc_workflow') {
      return Promise.reject(new Error(`unexpected command ${command}`));
    }
    switch (req.kind) {
      case 'list':
        return Promise.resolve({
          kind: 'list',
          data: { workflows, destinations: [] },
        });
      case 'get': {
        const workflow =
          options.workflow ?? workflows.find((item) => item.id === req.data?.id) ?? defaultWorkflow;
        return Promise.resolve({ kind: 'workflow', data: { workflow } });
      }
      case 'create':
        return Promise.resolve({
          kind: 'workflow',
          data: {
            workflow: {
              ...(req.data?.workflow ?? {}),
              id: 'save-quietly',
              isDefault: false,
            },
          },
        });
      case 'update':
        return Promise.resolve({
          kind: 'workflow',
          data: {
            workflow: {
              ...(req.data?.workflow ?? {}),
              id: req.data?.id,
              isDefault: req.data?.id === 'default',
            },
          },
        });
      case 'duplicate':
        return Promise.resolve({
          kind: 'workflow',
          data: {
            workflow: {
              ...annotate,
              id: 'annotate-and-share-2',
              name: 'Annotate and share copy',
              hotkey: null,
            },
          },
        });
      case 'delete':
        return Promise.resolve({ kind: 'deleted', data: { id: req.data?.id } });
      default:
        return Promise.reject(new Error(`unexpected request ${req.kind}`));
    }
  });
}

function route(path: string): void {
  const id = path === '/workflows/new' ? undefined : path.split('/').pop();
  currentRoute.set({
    path,
    page: 'workflow-editor',
    params: id ? { id } : {},
  });
}

function labels(container: HTMLElement): string[] {
  return Array.from(container.querySelectorAll('.step .label')).map(
    (element) => element.textContent ?? '',
  );
}

beforeEach(() => {
  mocks.invoke.mockReset();
  currentRoute.set({ path: '/workflows', page: 'workflows', params: {} });
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('Workflow editor', () => {
  it('renders the new workflow header and subtitle', async () => {
    route('/workflows/new');
    install();
    const { container } = render(WorkflowEditor);
    await screen.findByText('New workflow');
    await expectAccessible(container);
    expect(screen.getByText('Steps run in order after the hotkey is pressed.')).not.toBeNull();
    expect(screen.getByLabelText('Name')).not.toBeNull();
    expect(screen.getByLabelText('Hotkey')).not.toBeNull();
    expect(screen.getByLabelText('Enabled')).not.toBeNull();
    expect(screen.getByLabelText('Add step')).not.toBeNull();
  });

  it('adds every step type and edits the capture kind inline', async () => {
    route('/workflows/new');
    install();
    const { container } = render(WorkflowEditor);
    const add = (await screen.findByLabelText('Add step')) as HTMLSelectElement;
    await fireEvent.change(add, { target: { value: 'capture' } });
    await waitFor(() => {
      expect(labels(container)).toEqual(['Capture region']);
    });
    const kind = screen.getByLabelText('Capture kind') as HTMLSelectElement;
    expect(kind.value).toBe('region');
    await fireEvent.change(kind, { target: { value: 'window' } });
    expect(labels(container)).toEqual(['Capture window']);
  });

  it('reorders steps with Alt+ArrowDown on the handle', async () => {
    route('/workflows/new');
    install();
    const { container } = render(WorkflowEditor);
    const add = (await screen.findByLabelText('Add step')) as HTMLSelectElement;
    await fireEvent.change(add, { target: { value: 'capture' } });
    await fireEvent.change(add, { target: { value: 'save' } });
    expect(labels(container)).toEqual(['Capture region', 'Save']);
    await fireEvent.keyDown(screen.getByLabelText('Reorder: Capture region'), {
      key: 'ArrowDown',
      altKey: true,
    });
    expect(labels(container)).toEqual(['Save', 'Capture region']);
    await fireEvent.keyDown(screen.getByLabelText('Reorder: Capture region'), {
      key: 'ArrowUp',
      altKey: true,
    });
    expect(labels(container)).toEqual(['Capture region', 'Save']);
  });

  it('shows inline parameters for upload destination and copy link format', async () => {
    route('/workflows/new');
    install();
    render(WorkflowEditor);
    const add = (await screen.findByLabelText('Add step')) as HTMLSelectElement;
    await fireEvent.change(add, { target: { value: 'capture' } });
    await fireEvent.change(add, { target: { value: 'upload' } });
    await fireEvent.change(add, { target: { value: 'copy_link' } });
    const destination = screen.getByLabelText('Upload destination') as HTMLSelectElement;
    expect(destination.value).toBe('');
    expect(Array.from(destination.options).map((option) => option.textContent)).toEqual([
      'Default destination',
    ]);
    const format = screen.getByLabelText('Copy link format') as HTMLSelectElement;
    await fireEvent.change(format, { target: { value: 'markdown' } });
    expect(format.value).toBe('markdown');
  });

  it('blocks saving while validation fails', async () => {
    route('/workflows/new');
    install();
    render(WorkflowEditor);
    await fireEvent.click(await screen.findByRole('button', { name: 'Save' }));
    expect(mocks.invoke).not.toHaveBeenCalledWith('ipc_workflow', {
      req: expect.objectContaining({ kind: 'create' }),
    });
    expect(screen.getAllByText('The name must be 1 to 60 characters.').length).toBeGreaterThan(0);
    expect(
      screen.getAllByText('The first step must be a Capture or Record step.').length,
    ).toBeGreaterThan(0);
  });

  it('creates a workflow and returns to the list', async () => {
    route('/workflows/new');
    install();
    render(WorkflowEditor);
    await fireEvent.input(await screen.findByLabelText('Name'), {
      target: { value: 'Save quietly' },
    });
    const add = (await screen.findByLabelText('Add step')) as HTMLSelectElement;
    await fireEvent.change(add, { target: { value: 'capture' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => {
      expect(mocks.invoke).toHaveBeenCalledWith('ipc_workflow', {
        req: {
          kind: 'create',
          data: {
            workflow: {
              name: 'Save quietly',
              hotkey: null,
              enabled: true,
              steps: [{ step: 'capture', kind: 'region' }],
            },
          },
        },
      });
    });
    expect(window.location.hash).toBe('#/workflows');
  });

  it('checks a hotkey for clashes live', async () => {
    route('/workflows/new');
    install({ preview: 'external' });
    render(WorkflowEditor);
    await fireEvent.click(await screen.findByRole('button', { name: 'Change Hotkey' }));
    await fireEvent.keyDown(window, { key: 'A', ctrlKey: true });
    expect(await screen.findByText('Off: another app is using this shortcut.')).not.toBeNull();
    expect(mocks.invoke).toHaveBeenCalledWith('ipc_hotkeys', {
      req: {
        kind: 'preview',
        data: { owner: 'workflow:new', chord: 'Ctrl+A' },
      },
    });
  });

  it('loads an existing workflow and duplicates it', async () => {
    route('/workflows/annotate-and-share');
    install({ workflow: annotate });
    render(WorkflowEditor);
    expect(await screen.findByText('Annotate and share')).not.toBeNull();
    await fireEvent.click(screen.getByRole('button', { name: 'Duplicate' }));
    await waitFor(() => {
      expect(mocks.invoke).toHaveBeenCalledWith('ipc_workflow', {
        req: { kind: 'duplicate', data: { id: 'annotate-and-share' } },
      });
    });
    expect(window.location.hash).toBe('#/workflows/annotate-and-share-2');
  });

  it('asks before deleting and returns to the list', async () => {
    route('/workflows/annotate-and-share');
    install({ workflow: annotate });
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    render(WorkflowEditor);
    await fireEvent.click(await screen.findByRole('button', { name: 'Delete' }));
    await waitFor(() => {
      expect(mocks.invoke).toHaveBeenCalledWith('ipc_workflow', {
        req: { kind: 'delete', data: { id: 'annotate-and-share' } },
      });
    });
    expect(window.confirm).toHaveBeenCalledWith('Delete Annotate and share?');
    expect(window.location.hash).toBe('#/workflows');
  });

  it('does not delete when the confirm is cancelled', async () => {
    route('/workflows/annotate-and-share');
    install({ workflow: annotate });
    vi.spyOn(window, 'confirm').mockReturnValue(false);
    render(WorkflowEditor);
    await fireEvent.click(await screen.findByRole('button', { name: 'Delete' }));
    expect(mocks.invoke).not.toHaveBeenCalledWith('ipc_workflow', {
      req: expect.objectContaining({ kind: 'delete' }),
    });
  });

  it('guards the default workflow name, toggle, hotkey and delete', async () => {
    route('/workflows/default');
    install({ workflow: defaultWorkflow });
    render(WorkflowEditor);
    expect(await screen.findByText('Screenshot to link')).not.toBeNull();
    expect((screen.getByLabelText('Name') as HTMLInputElement).disabled).toBe(true);
    expect((screen.getByLabelText('Enabled') as HTMLInputElement).disabled).toBe(true);
    expect(screen.queryByLabelText('Hotkey')).toBeNull();
    expect((screen.getByRole('button', { name: 'Delete' }) as HTMLButtonElement).disabled).toBe(
      true,
    );
    expect(screen.queryByText('The first step must be a Capture or Record step.')).toBeNull();
  });
});
