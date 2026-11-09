import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { expectAccessible } from '../../../test/axe';
import GeneralPage from './GeneralPage.svelte';
import { resetSettings } from './settings';
import { settingsDto } from './settingsDto';
import type { StepDto } from '../../../lib/ipc/bindings/StepDto';
import type { WorkflowInfo } from '../../../lib/ipc/bindings/WorkflowInfo';
import type { WorkflowInput } from '../../../lib/ipc/bindings/WorkflowInput';

const ipc = vi.hoisted(() => ({
  ping: vi.fn(),
  get: vi.fn(),
  update: vi.fn(),
  previewName: vi.fn(),
  browseFolder: vi.fn(),
}));

const workflowIpc = vi.hoisted(() => ({
  get: vi.fn(),
  list: vi.fn(),
  update: vi.fn(),
}));

vi.mock('../../../lib/ipc/settings', () => ipc);
vi.mock('../../../lib/ipc/workflow', () => workflowIpc);

const DEFAULT_STEPS: StepDto[] = [
  { step: 'copy_image' },
  { step: 'save' },
  { step: 'upload', dest: null },
  { step: 'copy_link', format: 'direct' },
  { step: 'play_sound' },
];

const STEP_LABELS = [
  'Copy image to clipboard',
  'Open in editor to annotate',
  'Save to folder',
  'Upload to default destination',
  'Copy link to clipboard',
  'Play a sound',
];

function defaultWorkflow(steps: StepDto[] = DEFAULT_STEPS): WorkflowInfo {
  return {
    id: 'default',
    name: 'Screenshot to link',
    hotkey: null,
    enabled: true,
    steps,
    isDefault: true,
  };
}

function catalog(): WorkflowInfo[] {
  return [
    defaultWorkflow(),
    {
      id: 'annotate-and-share',
      name: 'Annotate and share',
      hotkey: 'Ctrl+Shift+A',
      enabled: true,
      steps: [],
      isDefault: false,
    },
    {
      id: 'bug-report-clip',
      name: 'Bug report clip',
      hotkey: 'Ctrl+Alt+R',
      enabled: true,
      steps: [],
      isDefault: false,
    },
    {
      id: 'grab-text',
      name: 'Grab text',
      hotkey: 'Ctrl+Alt+T',
      enabled: false,
      steps: [],
      isDefault: false,
    },
    {
      id: 'save-quietly',
      name: 'Save quietly',
      hotkey: 'Ctrl+Alt+PrtSc',
      enabled: false,
      steps: [],
      isDefault: false,
    },
  ];
}

beforeEach(() => {
  resetSettings();
  ipc.get.mockReset();
  ipc.update.mockReset();
  ipc.previewName.mockReset();
  ipc.browseFolder.mockReset();
  workflowIpc.get.mockReset();
  workflowIpc.list.mockReset();
  workflowIpc.update.mockReset();
  ipc.get.mockResolvedValue(settingsDto());
  ipc.update.mockResolvedValue(settingsDto());
  ipc.previewName.mockResolvedValue('chrome_2026-11-08_14-31-07.png');
  ipc.browseFolder.mockResolvedValue(null);
  workflowIpc.get.mockResolvedValue(defaultWorkflow());
  workflowIpc.list.mockResolvedValue({ workflows: catalog(), destinations: [] });
  workflowIpc.update.mockImplementation((id: string, input: WorkflowInput) =>
    Promise.resolve({ ...defaultWorkflow(input.steps), id }),
  );
});

afterEach(cleanup);

describe('GeneralPage', () => {
  it('gets the settings on mount and shows the defaults', async () => {
    const { container } = render(GeneralPage);
    await expectAccessible(container);
    expect(screen.getByText('General')).not.toBeNull();
    expect(screen.getByText('Snapline')).not.toBeNull();
    expect(screen.getByText('Version 0.1.0')).not.toBeNull();
    expect(screen.getByText('Launch at sign-in')).not.toBeNull();
    expect(screen.getByText('Save captures to')).not.toBeNull();
    expect((screen.getByLabelText('Save captures to') as HTMLInputElement).value).toBe(
      '%USERPROFILE%\\Pictures\\Snapline',
    );
    expect(screen.getByRole('radio', { name: 'PNG' }).getAttribute('aria-checked')).toBe('true');
    expect(screen.getByText('90%')).not.toBeNull();
    await waitFor(() => expect(ipc.get).toHaveBeenCalledTimes(1));
  });

  it('shows the name pattern preview from the service', async () => {
    render(GeneralPage);
    expect(await screen.findByText('chrome_2026-11-08_14-31-07.png')).not.toBeNull();
    expect(ipc.previewName).toHaveBeenCalledWith('{app}_{yyyy}-{MM}-{dd}_{HH}-{mm}-{ss}', 'png');
  });

  it('sends a text change after the 300 ms debounce', async () => {
    render(GeneralPage);
    await fireEvent.input(screen.getByLabelText('Name pattern'), {
      target: { value: '{app}_shot' },
    });
    await waitFor(() =>
      expect(ipc.update).toHaveBeenCalledWith({ general: { name_pattern: '{app}_shot' } }),
    );
  });

  it('sends a toggle change immediately', async () => {
    render(GeneralPage);
    await fireEvent.click(screen.getByRole('checkbox', { name: /Launch at sign-in/ }));
    await waitFor(() =>
      expect(ipc.update).toHaveBeenCalledWith({ general: { launch_at_signin: false } }),
    );
  });

  it('inserts a token into the pattern', async () => {
    render(GeneralPage);
    await screen.findByText('chrome_2026-11-08_14-31-07.png');
    await fireEvent.click(screen.getByRole('button', { name: '{n}' }));
    expect((screen.getByLabelText('Name pattern') as HTMLInputElement).value).toBe(
      '{app}_{yyyy}-{MM}-{dd}_{HH}-{mm}-{ss}{n}',
    );
    await waitFor(() => expect(ipc.update).toHaveBeenCalled());
  });

  it('asks the service for a folder and applies the pick', async () => {
    ipc.browseFolder.mockResolvedValue('C:\\Captures');
    render(GeneralPage);
    await fireEvent.click(screen.getByRole('button', { name: 'Browse…' }));
    await waitFor(() =>
      expect(ipc.update).toHaveBeenCalledWith({ general: { save_folder: 'C:\\Captures' } }),
    );
  });

  it('filters rows by description on the current page', () => {
    render(GeneralPage, { props: { filter: 'Organise saved files automatically' } });
    expect(screen.getByText('Subfolders')).not.toBeNull();
    expect(screen.queryByText('Launch at sign-in')).toBeNull();
  });
});

describe('GeneralPage default workflow', () => {
  it('shows the stored steps, fresh-install states and destination note', async () => {
    render(GeneralPage);
    const section = await screen.findByRole('region', { name: 'Default workflow · PrtSc' });
    await waitFor(() => expect(within(section).getAllByRole('checkbox')).toHaveLength(6));
    const boxes = within(section).getAllByRole('checkbox') as HTMLInputElement[];
    expect(boxes.map((box) => box.closest('label')?.textContent?.trim())).toEqual(STEP_LABELS);
    expect(boxes.map((box) => box.checked)).toEqual([true, false, true, true, true, true]);
    expect(within(section).getByText('Save locally only')).not.toBeNull();
    expect(workflowIpc.get).toHaveBeenCalledWith('default');
    expect(workflowIpc.update).not.toHaveBeenCalled();
  });

  it('adds and removes steps through Get and Update', async () => {
    render(GeneralPage);
    const annotate = (await screen.findByRole('checkbox', {
      name: 'Open in editor to annotate',
    })) as HTMLInputElement;
    await fireEvent.click(annotate);
    await waitFor(() => expect(workflowIpc.update).toHaveBeenCalledTimes(1));
    expect(workflowIpc.update.mock.calls[0][1].steps).toEqual([
      { step: 'copy_image' },
      { step: 'annotate' },
      { step: 'save' },
      { step: 'upload', dest: null },
      { step: 'copy_link', format: 'direct' },
      { step: 'play_sound' },
    ]);

    const copy = screen.getByRole('checkbox', {
      name: 'Copy image to clipboard',
    }) as HTMLInputElement;
    await fireEvent.click(copy);
    await waitFor(() => expect(workflowIpc.update).toHaveBeenCalledTimes(2));
    expect(workflowIpc.update.mock.calls[1][1].steps).toEqual([
      { step: 'annotate' },
      { step: 'save' },
      { step: 'upload', dest: null },
      { step: 'copy_link', format: 'direct' },
      { step: 'play_sound' },
    ]);
  });

  it('sends the new order when a row is reordered', async () => {
    render(GeneralPage);
    const handle = await screen.findByRole('button', { name: 'Reorder: Save to folder' });
    await fireEvent.keyDown(handle, { key: 'ArrowDown', altKey: true });
    await waitFor(() => expect(workflowIpc.update).toHaveBeenCalledTimes(1));
    expect(workflowIpc.update.mock.calls[0][1].steps).toEqual([
      { step: 'copy_image' },
      { step: 'upload', dest: null },
      { step: 'save' },
      { step: 'copy_link', format: 'direct' },
      { step: 'play_sound' },
    ]);
  });

  it('binds the copied link format to the setting and the copy link step', async () => {
    ipc.update.mockImplementation((patch: Record<string, unknown>) => {
      const dto = settingsDto();
      const general = patch.general as { copied_link_format?: typeof dto.general.copiedLinkFormat };
      if (general?.copied_link_format) {
        dto.general.copiedLinkFormat = general.copied_link_format;
      }
      return Promise.resolve(dto);
    });
    render(GeneralPage);
    await screen.findByRole('checkbox', { name: 'Copy image to clipboard' });
    const select = (await screen.findByRole('combobox', {
      name: 'Copied link format',
    })) as HTMLSelectElement;
    expect(select.value).toBe('Direct link');
    await fireEvent.change(select, { target: { value: 'Markdown' } });
    await waitFor(() =>
      expect(ipc.update).toHaveBeenCalledWith({ general: { copied_link_format: 'markdown' } }),
    );
    await waitFor(() => expect(workflowIpc.update).toHaveBeenCalledTimes(1));
    expect(workflowIpc.update.mock.calls[0][1].steps).toEqual([
      { step: 'copy_image' },
      { step: 'save' },
      { step: 'upload', dest: null },
      { step: 'copy_link', format: 'markdown' },
      { step: 'play_sound' },
    ]);
    await waitFor(() => expect(select.value).toBe('Markdown'));
  });

  it('shows the real workflow count and opens the Workflows tab', async () => {
    render(GeneralPage);
    const button = await screen.findByRole('button', { name: '5 workflows' });
    expect(
      screen.getByText('Per-hotkey workflows are edited in the app under Workflows'),
    ).not.toBeNull();
    await fireEvent.click(button);
    expect(window.location.hash).toBe('#/workflows');
  });

  it('shows the typed IPC message when a save fails', async () => {
    workflowIpc.update.mockRejectedValue({
      code: 'invalid',
      message: 'Copy link requires an earlier Upload step.',
    });
    render(GeneralPage);
    const save = (await screen.findByRole('checkbox', {
      name: 'Save to folder',
    })) as HTMLInputElement;
    await fireEvent.click(save);
    const alert = await screen.findByRole('alert');
    expect(alert.textContent).toBe('Copy link requires an earlier Upload step.');
  });
});
