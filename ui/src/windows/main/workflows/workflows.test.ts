import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { expectAccessible } from '../../../test/axe';

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));

import type { BlockedDto } from '../../../lib/ipc/bindings/BlockedDto';
import type { BindingDto } from '../../../lib/ipc/bindings/BindingDto';
import type { DestinationOption } from '../../../lib/ipc/bindings/DestinationOption';
import type { WorkflowInfo } from '../../../lib/ipc/bindings/WorkflowInfo';
import type { WorkflowInput } from '../../../lib/ipc/bindings/WorkflowInput';
import WorkflowsPage from './WorkflowsPage.svelte';
import {
  chordFor,
  clashLine,
  defaultWorkflowTooltip,
  displayName,
  stepLabel,
  validateWorkflow,
} from './data';

interface Request {
  kind: string;
  data?: {
    id?: string;
    workflow?: WorkflowInput;
  };
}

interface Options {
  workflows?: WorkflowInfo[];
  destinations?: DestinationOption[];
  bindings?: BindingDto[];
  blocked?: BlockedDto[];
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
  steps: [
    { step: 'capture', kind: 'region' },
    { step: 'annotate' },
    { step: 'upload', dest: null },
    { step: 'copy_link', format: 'markdown' },
  ],
  isDefault: false,
};

function install(options: Options = {}): void {
  const workflows = options.workflows ?? [defaultWorkflow, annotate];
  const bindings = options.bindings ?? [
    { owner: 'capture_region', label: 'Capture region', chord: 'PrtSc' },
    { owner: 'workflow:annotate-and-share', label: 'Workflow annotate', chord: 'Ctrl+Shift+A' },
  ];
  mocks.invoke.mockImplementation((command: string, args: { req: Request }) => {
    const req = args.req;
    if (command === 'ipc_hotkeys' && req.kind === 'get') {
      return Promise.resolve({
        kind: 'state',
        data: { bindings, blocked: options.blocked ?? [], paused: false },
      });
    }
    if (command === 'ipc_workflow' && req.kind === 'list') {
      return Promise.resolve({
        kind: 'list',
        data: { workflows, destinations: options.destinations ?? [] },
      });
    }
    if (command === 'ipc_workflow' && req.kind === 'update' && req.data?.id) {
      const data = req.data;
      const existing = workflows.find((workflow) => workflow.id === data.id);
      return Promise.resolve({
        kind: 'workflow',
        data: {
          workflow: {
            ...existing,
            ...(data.workflow ?? {}),
            id: data.id,
            isDefault: existing?.isDefault ?? false,
          },
        },
      });
    }
    return Promise.reject(new Error(`unexpected request ${command} ${req.kind}`));
  });
}

beforeEach(() => {
  mocks.invoke.mockReset();
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('Workflows page', () => {
  it('renders the real cards with names, chords and step pills', async () => {
    install();
    const { container } = render(WorkflowsPage);
    await screen.findByText('Screenshot to link (default)');
    await expectAccessible(container);
    expect(screen.getByText('Annotate and share')).not.toBeNull();
    expect(screen.getByText('PrtSc')).not.toBeNull();
    expect(screen.getByText('Ctrl+Shift+A')).not.toBeNull();
    expect(screen.getByText('Copy image')).not.toBeNull();
    expect(screen.getByText('Capture region')).not.toBeNull();
    expect(screen.getByText('Copy link')).not.toBeNull();
    expect(screen.getByRole('button', { name: 'New workflow' })).not.toBeNull();
  });

  it('disables the default toggle and explains why', async () => {
    install();
    render(WorkflowsPage);
    const toggle = (await screen.findByRole('checkbox', {
      name: 'Enable Screenshot to link (default)',
    })) as HTMLInputElement;
    expect(toggle.disabled).toBe(true);
    expect(toggle.checked).toBe(true);
    expect(toggle.getAttribute('title')).toBe(defaultWorkflowTooltip);
  });

  it('does not send an update when the default toggle is clicked', async () => {
    install();
    render(WorkflowsPage);
    const toggle = (await screen.findByRole('checkbox', {
      name: 'Enable Screenshot to link (default)',
    })) as HTMLInputElement;
    await fireEvent.click(toggle);
    expect(mocks.invoke).not.toHaveBeenCalledWith('ipc_workflow', {
      req: expect.objectContaining({ kind: 'update' }),
    });
  });

  it('toggles a normal workflow and applies the answer', async () => {
    install();
    render(WorkflowsPage);
    const toggle = (await screen.findByRole('checkbox', {
      name: 'Enable Annotate and share',
    })) as HTMLInputElement;
    await fireEvent.click(toggle);
    await waitFor(() => {
      expect(mocks.invoke).toHaveBeenCalledWith('ipc_workflow', {
        req: {
          kind: 'update',
          data: {
            id: 'annotate-and-share',
            workflow: {
              name: 'Annotate and share',
              hotkey: 'Ctrl+Shift+A',
              enabled: false,
              steps: annotate.steps,
            },
          },
        },
      });
    });
    await waitFor(() => {
      expect(screen.getByText('Off')).not.toBeNull();
    });
  });

  it('shows the internal clash text under the steps', async () => {
    install({
      workflows: [defaultWorkflow, annotate],
      bindings: [{ owner: 'capture_region', label: 'Capture region', chord: 'PrtSc' }],
      blocked: [
        {
          owner: 'workflow:annotate-and-share',
          label: 'Workflow annotate-and-share',
          chord: 'Ctrl+Shift+A',
          reason: 'internal',
        },
        {
          owner: 'capture_window',
          label: 'Capture window',
          chord: 'Ctrl+Shift+A',
          reason: 'internal',
        },
      ],
    });
    const { container } = render(WorkflowsPage);
    expect(
      await screen.findByText('Off because Ctrl+Shift+A is also used by Capture window.'),
    ).not.toBeNull();
    expect(container.querySelector('.chord.blocked')).not.toBeNull();
  });

  it('shows the empty state when there are no workflows', async () => {
    install({ workflows: [] });
    render(WorkflowsPage);
    expect(await screen.findByText('No workflows yet')).not.toBeNull();
  });

  it('opens the editor from a card', async () => {
    install();
    render(WorkflowsPage);
    const card = (await screen.findByText('Annotate and share')).closest('article');
    expect(card).not.toBeNull();
    await fireEvent.click(within(card as HTMLElement).getByRole('button', { name: 'Edit' }));
    expect(window.location.hash).toBe('#/workflows/annotate-and-share');
  });
});

describe('workflow card helpers', () => {
  const live = {
    bindings: [
      { owner: 'capture_region', label: 'Capture region', chord: 'PrtSc' },
      { owner: 'workflow:annotate-and-share', label: 'Workflow annotate', chord: 'Ctrl+Shift+A' },
    ],
    blocked: [],
    paused: false,
  };

  it('appends the default marker and finds the live chord', () => {
    expect(displayName(defaultWorkflow)).toBe('Screenshot to link (default)');
    expect(displayName(annotate)).toBe('Annotate and share');
    expect(chordFor(defaultWorkflow, live)).toBe('PrtSc');
    expect(chordFor(annotate, live)).toBe('Ctrl+Shift+A');
  });

  it('explains an internal clash and ignores external ones', () => {
    expect(clashLine(annotate, live)).toBeNull();
    const blocked = {
      ...live,
      blocked: [
        {
          owner: 'workflow:annotate-and-share',
          label: 'Workflow annotate-and-share',
          chord: 'Ctrl+Shift+A',
          reason: 'internal' as const,
        },
        {
          owner: 'capture_window',
          label: 'Capture window',
          chord: 'Ctrl+Shift+A',
          reason: 'internal' as const,
        },
      ],
    };
    expect(clashLine(annotate, blocked)).toBe(
      'Off because Ctrl+Shift+A is also used by Capture window.',
    );
    expect(
      clashLine(annotate, {
        ...live,
        blocked: [
          {
            owner: 'workflow:annotate-and-share',
            label: 'Workflow annotate-and-share',
            chord: 'Ctrl+Shift+A',
            reason: 'external' as const,
          },
        ],
      }),
    ).toBeNull();
  });

  it('uses the editor labels for every step', () => {
    expect(stepLabel({ step: 'capture', kind: 'last_region' })).toBe('Capture last region');
    expect(stepLabel({ step: 'record', kind: 'gif' })).toBe('Record GIF');
    expect(stepLabel({ step: 'extract_text' })).toBe('Extract text');
    expect(stepLabel({ step: 'play_sound' })).toBe('Play sound');
  });
});

describe('workflow validation', () => {
  it('accepts a capture workflow and reports the exact strings', () => {
    expect(validateWorkflow('Save quietly', [{ step: 'capture', kind: 'screen' }], false)).toEqual(
      [],
    );
    expect(
      validateWorkflow(
        'Annotate',
        [{ step: 'copy_link', format: 'direct' }, { step: 'convert_to_gif' }],
        false,
      ),
    ).toEqual([
      { step: 0, message: 'The first step must be a Capture or Record step.' },
      { step: 0, message: 'Copy link requires an earlier Upload step.' },
      { step: 1, message: 'Convert to GIF requires an earlier Record step.' },
    ]);
  });

  it('requires a name of one to sixty characters', () => {
    expect(validateWorkflow('', [], false)).toEqual([
      { step: null, message: 'The name must be 1 to 60 characters.' },
      { step: null, message: 'The first step must be a Capture or Record step.' },
    ]);
    expect(validateWorkflow('a'.repeat(61), [], false)[0].message).toBe(
      'The name must be 1 to 60 characters.',
    );
    expect(validateWorkflow('a'.repeat(60), [], true)).toEqual([]);
  });

  it('lets the default workflow start with any step', () => {
    expect(validateWorkflow('Screenshot to link', [{ step: 'copy_image' }], true)).toEqual([]);
  });
});
