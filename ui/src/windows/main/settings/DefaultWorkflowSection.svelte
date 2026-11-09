<script lang="ts">
  import { onMount } from 'svelte';
  import Button from '../../../components/Button.svelte';
  import Icon from '../../../components/Icon.svelte';
  import SelectRow from '../../../components/SelectRow.svelte';
  import StepList from '../../../components/StepList.svelte';
  import type { StepItem } from '../../../components/types';
  import type { DestinationOption } from '../../../lib/ipc/bindings/DestinationOption';
  import type { LinkFormatDto } from '../../../lib/ipc/bindings/LinkFormatDto';
  import type { StepDto } from '../../../lib/ipc/bindings/StepDto';
  import type { WorkflowInfo } from '../../../lib/ipc/bindings/WorkflowInfo';
  import type { WorkflowInput } from '../../../lib/ipc/bindings/WorkflowInput';
  import {
    get as getWorkflow,
    list as listWorkflows,
    update as updateWorkflow,
  } from '../../../lib/ipc/workflow';
  import type { IconName } from '../../../lib/icons';
  import { navigate } from '../../../router';
  import { LINK_FORMAT_OPTIONS, settings, updateGeneral } from './settings';
  import SettingsSection from './SettingsSection.svelte';

  interface Props {
    filter?: string;
  }

  let { filter = '' }: Props = $props();

  type KnownKind = 'copy_image' | 'annotate' | 'save' | 'upload' | 'copy_link' | 'play_sound';

  interface StepMeta {
    kind: KnownKind;
    label: string;
    icon: IconName;
  }

  const STEP_META: Record<string, StepMeta> = {
    copy: { kind: 'copy_image', label: 'Copy image to clipboard', icon: 'copy' },
    edit: { kind: 'annotate', label: 'Open in editor to annotate', icon: 'edit' },
    save: { kind: 'save', label: 'Save to folder', icon: 'folder' },
    upload: { kind: 'upload', label: 'Upload to default destination', icon: 'upload' },
    link: { kind: 'copy_link', label: 'Copy link to clipboard', icon: 'link' },
    sound: { kind: 'play_sound', label: 'Play a sound', icon: 'speaker' },
  };

  const ROW_ORDER = ['copy', 'edit', 'save', 'upload', 'link', 'sound'];

  const LINK_FORMAT_VALUES: Record<string, LinkFormatDto> = {
    'Direct link': 'direct',
    Markdown: 'markdown',
    'HTML <img>': 'html',
    BBCode: 'bbcode',
    'Page link': 'page',
  };

  let workflow = $state<WorkflowInfo | null>(null);
  let destinations = $state<DestinationOption[]>([]);
  let count = $state(0);
  let rows = $state<StepItem[]>([]);
  let message = $state('');

  const destinationNote = $derived(destinationName($settings.uploads.images, destinations));

  $effect(() => {
    const note = destinationNote;
    if (!rows.some((row) => row.id === 'upload' && row.note !== note)) {
      return;
    }
    rows = rows.map((row) => (row.id === 'upload' ? { ...row, note } : row));
  });

  onMount(() => {
    void load();
  });

  async function load(): Promise<void> {
    try {
      const current = await getWorkflow('default');
      workflow = current;
      rows = buildRows(current.steps);
    } catch (error) {
      message = messageOf(error);
    }
    try {
      const all = await listWorkflows();
      destinations = all.destinations;
      count = all.workflows.length;
    } catch (error) {
      if (message === '') {
        message = messageOf(error);
      }
    }
  }

  function knownKind(step: StepDto): KnownKind | null {
    for (const id of ROW_ORDER) {
      if (STEP_META[id].kind === step.step) {
        return STEP_META[id].kind;
      }
    }
    return null;
  }

  function destinationName(id: string, options: DestinationOption[]): string {
    if (id === 'save_locally_only' || id === 'Save locally only') {
      return 'Save locally only';
    }
    if (id === 'none' || id === 'None') {
      return 'None';
    }
    return options.find((option) => option.id === id || option.name === id)?.name ?? id;
  }

  function buildRows(steps: StepDto[]): StepItem[] {
    const included = ROW_ORDER.filter((id) =>
      steps.some((step) => step.step === STEP_META[id].kind),
    );
    const order = [...included];
    for (const id of ROW_ORDER) {
      if (order.includes(id)) {
        continue;
      }
      const canonical = ROW_ORDER.indexOf(id);
      let at = order.findIndex((existing) => ROW_ORDER.indexOf(existing) > canonical);
      if (at < 0) {
        at = order.length;
      }
      order.splice(at, 0, id);
    }
    return order.map((id) => ({
      id,
      label: STEP_META[id].label,
      icon: STEP_META[id].icon,
      checked: steps.some((step) => step.step === STEP_META[id].kind),
      note: id === 'upload' ? destinationNote : undefined,
    }));
  }

  function stepFor(id: string, current: WorkflowInfo): StepDto {
    const kind = STEP_META[id].kind;
    const existing = current.steps.find((step) => step.step === kind);
    if (existing) {
      return existing;
    }
    switch (kind) {
      case 'upload':
        return { step: 'upload', dest: null };
      case 'copy_link':
        return { step: 'copy_link', format: linkFormatValue($settings.general.copiedLinkFormat) };
      default:
        return { step: kind };
    }
  }

  function extraSteps(current: WorkflowInfo): { step: StepDto; index: number }[] {
    return current.steps
      .map((step, index) => ({ step, index }))
      .filter(({ step }) => knownKind(step) === null);
  }

  function buildSteps(items: StepItem[], current: WorkflowInfo): StepDto[] {
    const steps = items.filter((item) => item.checked).map((item) => stepFor(item.id, current));
    for (const extra of extraSteps(current)) {
      const at = Math.min(extra.index, steps.length);
      steps.splice(at, 0, extra.step);
    }
    return steps;
  }

  function workflowInput(current: WorkflowInfo, steps: StepDto[]): WorkflowInput {
    return {
      name: current.name,
      hotkey: current.hotkey,
      enabled: current.enabled,
      steps,
    };
  }

  function linkFormatValue(label: string): LinkFormatDto {
    return LINK_FORMAT_VALUES[label] ?? 'direct';
  }

  function messageOf(error: unknown): string {
    if (typeof error === 'object' && error !== null && 'message' in error) {
      const value = (error as { message?: unknown }).message;
      if (typeof value === 'string' && value !== '') {
        return value;
      }
    }
    return 'The default workflow could not be saved.';
  }

  function applyRows(items: StepItem[]): void {
    const current = workflow;
    rows = items;
    if (current === null) {
      return;
    }
    void persist(current, buildSteps(items, current));
  }

  function changeLinkFormat(label: string): void {
    updateGeneral('copiedLinkFormat', label);
    const current = workflow;
    if (current === null || !current.steps.some((step) => step.step === 'copy_link')) {
      return;
    }
    const steps = current.steps.map((step) =>
      step.step === 'copy_link' ? { step: 'copy_link', format: linkFormatValue(label) } : step,
    );
    void persist(current, steps);
  }

  async function persist(current: WorkflowInfo, steps: StepDto[]): Promise<void> {
    try {
      const updated = await updateWorkflow('default', workflowInput(current, steps));
      workflow = updated;
      rows = buildRows(updated.steps);
      message = '';
    } catch (error) {
      message = messageOf(error);
      rows = buildRows(current.steps);
    }
  }

  function matches(title: string, description = ''): boolean {
    const needle = filter.trim().toLowerCase();
    return (
      needle === '' ||
      title.toLowerCase().includes(needle) ||
      description.toLowerCase().includes(needle)
    );
  }

  const stepsVisible = $derived(
    matches(
      'Default workflow · PrtSc',
      'What happens after a capture. Each hotkey can run its own workflow — this is the one PrtSc and the tray use.',
    ) ||
      matches('Copy image to clipboard') ||
      matches('Open in editor to annotate') ||
      matches('Save to folder') ||
      matches('Upload to default destination') ||
      matches('Copy link to clipboard') ||
      matches('Play a sound'),
  );
  const linkFormatVisible = $derived(
    matches('Copied link format', 'What gets pasted when you press Ctrl+V'),
  );
  const otherWorkflowsVisible = $derived(
    matches('Other workflows', 'Per-hotkey workflows are edited in the app under Workflows'),
  );
  const visible = $derived(stepsVisible || linkFormatVisible || otherWorkflowsVisible);
</script>

{#if visible}
  <SettingsSection
    title="Default workflow · PrtSc"
    description="What happens after a capture. Each hotkey can run its own workflow — this is the one PrtSc and the tray use."
  >
    {#if message}
      <p class="message" role="alert">{message}</p>
    {/if}
    {#if stepsVisible}
      <StepList bind:items={rows} onchange={applyRows} />
    {/if}
    {#if linkFormatVisible}
      <SelectRow
        icon="code"
        title="Copied link format"
        description="What gets pasted when you press Ctrl+V"
        options={LINK_FORMAT_OPTIONS}
        value={$settings.general.copiedLinkFormat}
        onchange={changeLinkFormat}
      />
    {/if}
    {#if otherWorkflowsVisible}
      <div class="action-row">
        <Icon name="gear" size={20} />
        <div class="action-text">
          <span class="action-title">Other workflows</span>
          <span class="action-description"
            >Per-hotkey workflows are edited in the app under Workflows</span
          >
        </div>
        <Button variant="secondary" height={34} onclick={() => navigate('/workflows')}>
          {count} workflows
        </Button>
      </div>
    {/if}
  </SettingsSection>
{/if}

<style>
  .message {
    margin: 0;
    padding: 10px 12px;
    border: 1px solid var(--err);
    border-radius: 4px;
    color: var(--err);
    font-size: 12px;
  }

  .action-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 12px 16px;
    min-height: 68px;
    padding: 12px 20px;
    background: var(--card);
    border: 1px solid var(--stroke);
    border-radius: 6px;
  }

  .action-text {
    display: flex;
    flex: 1 1 220px;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .action-title {
    font-size: 14px;
  }

  .action-description {
    font-size: 12px;
    color: var(--text3);
  }
</style>
