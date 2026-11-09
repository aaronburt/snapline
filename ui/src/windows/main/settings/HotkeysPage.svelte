<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from '../../../components/Icon.svelte';
  import HotkeyField from '../../../components/HotkeyField.svelte';
  import ToggleRow from '../../../components/ToggleRow.svelte';
  import type { IconName } from '../../../lib/icons';
  import type { PreviewDto } from '../../../lib/ipc/bindings/PreviewDto';
  import { get as getHotkeys, preview as previewHotkey } from '../../../lib/ipc/hotkeys';
  import type { HotkeysState } from '../../../lib/ipc/hotkeys';
  import SettingsPage from './SettingsPage.svelte';
  import SettingsSection from './SettingsSection.svelte';
  import { loadSettings, settings, updateHotkeys } from './settings';
  import type { HotkeySettings } from './settings';

  interface Props {
    filter?: string;
  }

  let { filter = '' }: Props = $props();

  type ChordKey = Exclude<keyof HotkeySettings, 'takeOverPrtSc' | 'warnClashes'>;

  interface Row {
    key: ChordKey;
    owner: string;
    label: string;
    description: string;
    icon: IconName;
  }

  interface Group {
    title: string;
    rows: Row[];
  }

  const GROUPS: Group[] = [
    {
      title: 'Capture',
      rows: [
        {
          key: 'captureRegion',
          owner: 'capture_region',
          label: 'Capture region',
          description: 'Runs the default workflow',
          icon: 'region',
        },
        {
          key: 'captureWindow',
          owner: 'capture_window',
          label: 'Capture window',
          description: '',
          icon: 'window',
        },
        {
          key: 'captureFullScreen',
          owner: 'capture_screen',
          label: 'Capture full screen',
          description: '',
          icon: 'screen',
        },
        {
          key: 'scrollingCapture',
          owner: 'capture_scrolling',
          label: 'Scrolling capture',
          description: '',
          icon: 'scroll',
        },
        {
          key: 'lastRegion',
          owner: 'capture_last_region',
          label: 'Last region again',
          description: '',
          icon: 'stop',
        },
      ],
    },
    {
      title: 'Recording',
      rows: [
        {
          key: 'recordVideo',
          owner: 'record_video',
          label: 'Record video',
          description: '',
          icon: 'film',
        },
        {
          key: 'recordGif',
          owner: 'record_gif',
          label: 'Record GIF',
          description: '',
          icon: 'gifMark',
        },
      ],
    },
    {
      title: 'Tools',
      rows: [
        {
          key: 'textCapture',
          owner: 'tool_ocr',
          label: 'Text capture (OCR)',
          description: '',
          icon: 'ocr',
        },
        {
          key: 'colourPicker',
          owner: 'tool_colour',
          label: 'Colour picker',
          description: '',
          icon: 'palette',
        },
        {
          key: 'screenRuler',
          owner: 'tool_ruler',
          label: 'Screen ruler',
          description: '',
          icon: 'ruler',
        },
        {
          key: 'uploadClipboard',
          owner: 'upload_clipboard',
          label: 'Upload from clipboard',
          description: '',
          icon: 'clip',
        },
      ],
    },
  ];

  const OWNER_LABELS: Record<string, string> = Object.fromEntries(
    GROUPS.flatMap((group) => group.rows.map((row) => [row.owner, row.label] as const)),
  );

  const EXTERNAL_OFF = 'Off: another app is using this shortcut.';
  const PRTSC_HINT = 'Sign out and back in for Windows to release the PrtSc key.';

  let live = $state<HotkeysState>({ bindings: [], blocked: [], paused: false });
  let previews = $state<Record<string, string>>({});
  let capturing = $state<string | null>(null);
  let prtscHint = $state(false);
  let prtscHintShown = false;
  let request = 0;

  const bound = $derived(
    new Map(live.bindings.map((binding) => [binding.owner, binding.chord] as const)),
  );

  const blockedText = $derived.by(() => {
    const grouped = new Map<string, string[]>();
    for (const entry of live.blocked) {
      const owners = grouped.get(entry.chord) ?? [];
      owners.push(entry.owner);
      grouped.set(entry.chord, owners);
    }
    const text: Record<string, string> = {};
    for (const entry of live.blocked) {
      if (entry.reason === 'external') {
        text[entry.owner] = EXTERNAL_OFF;
        continue;
      }
      const others = (grouped.get(entry.chord) ?? [])
        .filter((owner) => owner !== entry.owner)
        .map(ownerLabel);
      text[entry.owner] = `Off: also used by ${formatOwners(others, entry.label)}.`;
    }
    return text;
  });

  onMount(() => {
    void initialise();
  });

  async function initialise(): Promise<void> {
    await loadSettings();
    await refresh();
  }

  async function refresh(): Promise<void> {
    try {
      live = await getHotkeys();
    } catch {
      return;
    }
  }

  function ownerLabel(owner: string): string {
    if (OWNER_LABELS[owner]) {
      return OWNER_LABELS[owner];
    }
    if (owner.startsWith('workflow:')) {
      return `Workflow ${owner.slice('workflow:'.length)}`;
    }
    return owner;
  }

  function formatOwners(labels: string[], fallback: string): string {
    if (labels.length === 0) {
      return fallback;
    }
    if (labels.length === 1) {
      return labels[0];
    }
    if (labels.length === 2) {
      return `${labels[0]} and ${labels[1]}`;
    }
    return `${labels.slice(0, -1).join(', ')} and ${labels[labels.length - 1]}`;
  }

  function previewText(preview: PreviewDto): string {
    if (preview === 'free') {
      return '';
    }
    if (preview === 'external') {
      return EXTERNAL_OFF;
    }
    const owners = preview.internal.owners.map(ownerLabel);
    return `Off: also used by ${formatOwners(owners, '')}.`;
  }

  function chordFor(row: Row): string {
    return bound.get(row.owner) ?? $settings.hotkeys[row.key];
  }

  function rowError(row: Row): string {
    const preview = previews[row.owner];
    if (preview !== undefined) {
      return preview;
    }
    return blockedText[row.owner] ?? '';
  }

  function clearPreview(owner: string): void {
    const next = { ...previews };
    delete next[owner];
    previews = next;
  }

  async function showPreview(owner: string, chord: string): Promise<void> {
    const mine = ++request;
    try {
      const preview = await previewHotkey(owner, chord);
      if (mine === request) {
        previews = { ...previews, [owner]: previewText(preview) };
      }
    } catch {
      if (mine === request) {
        clearPreview(owner);
      }
    }
  }

  function beginCapture(event: MouseEvent): void {
    const target = event.target as HTMLElement | null;
    const button = target?.closest('button');
    if (!button || button.textContent?.trim() !== 'Change') {
      return;
    }
    const ownerRow = button.closest('[data-owner]');
    if (!(ownerRow instanceof HTMLElement) || !ownerRow.dataset.owner) {
      return;
    }
    capturing = ownerRow.dataset.owner;
    previews = { ...previews, [capturing]: '' };
  }

  function keyName(key: string): string {
    if (key === 'PrintScreen') {
      return 'PrtSc';
    }
    if (key === ' ') {
      return 'Space';
    }
    if (key.length === 1) {
      return key.toUpperCase();
    }
    return key;
  }

  type CaptureResult =
    | { kind: 'cancel' }
    | { kind: 'clear' }
    | { kind: 'chord'; chord: string };

  function captureResult(event: KeyboardEvent): CaptureResult | null {
    if (event.key === 'Escape') {
      return { kind: 'cancel' };
    }
    if (event.key === 'Backspace') {
      return { kind: 'clear' };
    }
    if (['Control', 'Alt', 'Shift', 'Meta'].includes(event.key)) {
      return null;
    }
    const name = keyName(event.key);
    const hasModifier = event.ctrlKey || event.altKey || event.shiftKey || event.metaKey;
    const allowedAlone =
      name === 'PrtSc' || name === 'Pause' || /^F([1-9]|1[0-9]|2[0-4])$/.test(name);
    if (!hasModifier && !allowedAlone) {
      return null;
    }
    if (event.metaKey && !event.shiftKey) {
      return null;
    }
    const chord: string[] = [];
    if (event.ctrlKey) {
      chord.push('Ctrl');
    }
    if (event.altKey) {
      chord.push('Alt');
    }
    if (event.shiftKey) {
      chord.push('Shift');
    }
    if (event.metaKey) {
      chord.push('Win');
    }
    chord.push(name);
    return { kind: 'chord', chord: chord.join('+') };
  }

  function handleKeydown(event: KeyboardEvent): void {
    if (capturing === null) {
      return;
    }
    const result = captureResult(event);
    if (!result) {
      return;
    }
    const owner = capturing;
    capturing = null;
    if (result.kind === 'chord') {
      void showPreview(owner, result.chord);
    } else {
      clearPreview(owner);
    }
  }

  async function commit(key: ChordKey, chord: string): Promise<void> {
    updateHotkeys(key, chord);
    await loadSettings();
    await refresh();
  }

  function changePrtSc(value: boolean): void {
    updateHotkeys('takeOverPrtSc', value);
    if (!value) {
      prtscHint = false;
      return;
    }
    if (!prtscHintShown) {
      prtscHintShown = true;
      prtscHint = true;
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

  function groupVisible(group: Group): boolean {
    return (
      matches(group.title) ||
      group.rows.some((row) => matches(row.label, row.description))
    );
  }

  const takeOverVisible = $derived(
    matches('Take over the PrtSc key', 'Stops Windows opening Snipping Tool when you press PrtSc'),
  );
  const clashVisible = $derived(
    matches('Warn about clashes', 'Tell me when another app already uses a hotkey'),
  );
  const showBehaviour = $derived(matches('Behaviour') || takeOverVisible || clashVisible);
</script>

<svelte:window onclick={beginCapture} onkeydown={handleKeydown} />

<SettingsPage
  title="Hotkeys"
  intro="If two actions share a chord, or another app owns it, neither is bound and you are told."
>
  {#each GROUPS as group (group.title)}
    {#if groupVisible(group)}
      <SettingsSection title={group.title}>
        {#each group.rows as row (row.owner)}
          {#if matches(row.label, row.description)}
            <div class="row" class:blocked={rowError(row) !== ''} data-owner={row.owner}>
              <Icon name={row.icon} size={20} />
              <div class="text">
                <span class="title">{row.label}</span>
                {#if row.description}
                  <span class="description">{row.description}</span>
                {/if}
              </div>
              <div class="control">
                <HotkeyField
                  label={row.label}
                  value={chordFor(row)}
                  clash={rowError(row)}
                  onchange={(value) => void commit(row.key, value)}
                />
              </div>
            </div>
          {/if}
        {/each}
      </SettingsSection>
    {/if}
  {/each}

  {#if showBehaviour}
    <SettingsSection title="Behaviour">
      {#if takeOverVisible}
        <ToggleRow
          icon="shield"
          title="Take over the PrtSc key"
          description="Stops Windows opening Snipping Tool when you press PrtSc"
          checked={$settings.hotkeys.takeOverPrtSc}
          onchange={changePrtSc}
        />
        {#if prtscHint}
          <p class="hint" role="status">{PRTSC_HINT}</p>
        {/if}
      {/if}
      {#if clashVisible}
        <ToggleRow
          icon="alert"
          title="Warn about clashes"
          description="Tell me when another app already uses a hotkey"
          checked={$settings.hotkeys.warnClashes}
          onchange={(value) => updateHotkeys('warnClashes', value)}
        />
      {/if}
    </SettingsSection>
  {/if}
</SettingsPage>

<style>
  .row {
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

  .row.blocked :global(input) {
    border-color: var(--err);
  }

  .text {
    display: flex;
    flex: 1 1 220px;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .title {
    font-size: 14px;
  }

  .description {
    font-size: 12px;
    color: var(--text3);
  }

  .control {
    flex: 1 1 320px;
    max-width: 460px;
  }

  .hint {
    margin: 0;
    padding: 0 20px;
    font-size: 12px;
    color: var(--text3);
  }
</style>
