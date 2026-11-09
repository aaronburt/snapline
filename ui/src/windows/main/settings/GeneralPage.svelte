<script lang="ts">
  import { onMount, tick } from 'svelte';
  import Logo from '../../../components/Logo.svelte';
  import Card from '../../../components/Card.svelte';
  import Icon from '../../../components/Icon.svelte';
  import Button from '../../../components/Button.svelte';
  import ToggleRow from '../../../components/ToggleRow.svelte';
  import SelectRow from '../../../components/SelectRow.svelte';
  import SegmentedRow from '../../../components/SegmentedRow.svelte';
  import RangeRow from '../../../components/RangeRow.svelte';
  import FieldRow from '../../../components/FieldRow.svelte';
  import StepList from '../../../components/StepList.svelte';
  import type { StepItem } from '../../../components/types';
  import type { IconName } from '../../../lib/icons';
  import { navigate } from '../../../router';
  import SettingsPage from './SettingsPage.svelte';
  import SettingsSection from './SettingsSection.svelte';
  import {
    IMAGE_FORMAT_OPTIONS,
    LINK_FORMAT_OPTIONS,
    NAME_TOKENS,
    SUBFOLDER_OPTIONS,
    imageFormatValue,
    loadSettings,
    pickFolder,
    requestPreview,
    settings,
    updateGeneral,
    updateWorkflow,
  } from './settings';

  interface Props {
    filter?: string;
  }

  let { filter = '' }: Props = $props();

  const rows = $derived($settings.general);
  let preview = $state('');
  let previewTimer: ReturnType<typeof setTimeout> | undefined;
  let patternInput = $state<HTMLInputElement>();

  onMount(() => {
    void loadSettings();
  });

  $effect(() => {
    const pattern = rows.namePattern;
    const format = imageFormatValue(rows.imageFormat);
    if (previewTimer !== undefined) {
      clearTimeout(previewTimer);
    }
    previewTimer = setTimeout(() => {
      void requestPreview(pattern, format).then((name) => {
        preview = name;
      });
    }, 300);
    return () => {
      if (previewTimer !== undefined) {
        clearTimeout(previewTimer);
      }
    };
  });

  let steps = $state<StepItem[]>([]);

  interface StepMeta {
    id: string;
    label: string;
    icon: IconName;
    note?: string;
  }

  const STEP_META: Record<string, StepMeta> = {
    copy_image: { id: 'copy', label: 'Copy image to clipboard', icon: 'copy' },
    annotate: { id: 'edit', label: 'Open in editor to annotate', icon: 'edit' },
    save: { id: 'save', label: 'Save to folder', icon: 'folder' },
    upload: {
      id: 'upload',
      label: 'Upload to default destination',
      icon: 'upload',
      note: 'Save locally only',
    },
    copy_link: { id: 'link', label: 'Copy link to clipboard', icon: 'link' },
    play_sound: { id: 'sound', label: 'Play a sound', icon: 'speaker' },
  };

  const STEP_KINDS: Record<string, string> = {
    copy: 'copy_image',
    edit: 'annotate',
    save: 'save',
    upload: 'upload',
    link: 'copy_link',
    sound: 'play_sound',
  };

  $effect(() => {
    steps = $settings.general.defaultWorkflow.map((step, index) => {
      const meta = STEP_META[step.kind];
      return {
        id: meta?.id ?? `step-${index}`,
        label: meta?.label ?? step.kind,
        icon: meta?.icon ?? 'gear',
        checked: step.enabled,
        note: meta?.note,
      };
    });
  });

  function applySteps(items: StepItem[]): void {
    updateWorkflow(
      items.map((item) => ({
        kind: STEP_KINDS[item.id] ?? item.id,
        enabled: item.checked ?? false,
      })),
    );
  }

  const workflowCount = 5;

  function matches(title: string, description = ''): boolean {
    const needle = filter.trim().toLowerCase();
    return (
      needle === '' ||
      title.toLowerCase().includes(needle) ||
      description.toLowerCase().includes(needle)
    );
  }

  function addToken(token: string): void {
    const input = patternInput;
    const pattern = rows.namePattern;
    const focused = input !== undefined && document.activeElement === input;
    const start = focused && input ? (input.selectionStart ?? pattern.length) : pattern.length;
    const end = focused && input ? (input.selectionEnd ?? start) : start;
    const next = pattern.slice(0, start) + token + pattern.slice(end);
    updateGeneral('namePattern', next);
    void tick().then(() => {
      input?.focus();
      input?.setSelectionRange(start + token.length, start + token.length);
    });
  }

  async function browseSaveFolder(): Promise<void> {
    const path = await pickFolder(rows.saveFolder);
    if (path !== null) {
      updateGeneral('saveFolder', path);
    }
  }

  const versionVisible = $derived(matches('Snapline', 'Version 0.1.0'));

  const launchVisible = $derived(
    matches('Launch at sign-in', 'Start Snapline in the tray when you sign in to Windows'),
  );
  const minimiseVisible = $derived(
    matches('Start minimised to tray', "Don't open the main window on launch"),
  );
  const notificationsVisible = $derived(
    matches('Notifications', 'Show a toast when an upload finishes or a link is copied'),
  );
  const showStartup = $derived(
    matches('Startup') || launchVisible || minimiseVisible || notificationsVisible,
  );

  const folderVisible = $derived(
    matches('Save captures to', 'Captures are kept on this PC even after they are uploaded'),
  );
  const subfoldersVisible = $derived(
    matches('Subfolders', 'Organise saved files automatically'),
  );
  const formatVisible = $derived(
    matches('Image format', 'PNG is lossless; the others are smaller'),
  );
  const qualityVisible = $derived(matches('Lossy quality', 'Used for JPEG and WebP'));
  const showSaving = $derived(
    matches('Saving') || folderVisible || subfoldersVisible || formatVisible || qualityVisible,
  );

  const namingVisible = $derived(
    matches('File naming', 'Click a token to add it. Tokens are filled in at the moment of capture.') ||
      matches('Name pattern') ||
      matches('Preview'),
  );

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
  const showWorkflow = $derived(stepsVisible || linkFormatVisible || otherWorkflowsVisible);
</script>

<SettingsPage title="General">
  {#if versionVisible}
    <Card>
      <div class="version">
        <Logo size={48} />
        <div class="version-text">
          <span class="version-name">Snapline</span>
          <span class="version-number">Version 0.1.0</span>
        </div>
      </div>
    </Card>
  {/if}

  {#if showStartup}
    <SettingsSection title="Startup">
      {#if launchVisible}
        <ToggleRow
          icon="power"
          title="Launch at sign-in"
          description="Start Snapline in the tray when you sign in to Windows"
          checked={rows.launchAtSignIn}
          onchange={(value) => updateGeneral('launchAtSignIn', value)}
        />
      {/if}
      {#if minimiseVisible}
        <ToggleRow
          icon="tray"
          title="Start minimised to tray"
          description="Don't open the main window on launch"
          checked={rows.startMinimised}
          onchange={(value) => updateGeneral('startMinimised', value)}
        />
      {/if}
      {#if notificationsVisible}
        <ToggleRow
          icon="bell"
          title="Notifications"
          description="Show a toast when an upload finishes or a link is copied"
          checked={rows.notifications}
          onchange={(value) => updateGeneral('notifications', value)}
        />
      {/if}
    </SettingsSection>
  {/if}

  {#if showSaving}
    <SettingsSection title="Saving">
      {#if folderVisible}
        <FieldRow
          icon="folder"
          title="Save captures to"
          description="Captures are kept on this PC even after they are uploaded"
          mono
          button="Browse…"
          value={rows.saveFolder}
          onchange={(value) => updateGeneral('saveFolder', value)}
          onbutton={browseSaveFolder}
        />
      {/if}
      {#if subfoldersVisible}
        <SelectRow
          icon="tree"
          title="Subfolders"
          description="Organise saved files automatically"
          options={SUBFOLDER_OPTIONS}
          value={rows.subfolders}
          onchange={(value) => updateGeneral('subfolders', value)}
        />
      {/if}
      {#if formatVisible}
        <SegmentedRow
          icon="image"
          title="Image format"
          description="PNG is lossless; the others are smaller"
          options={IMAGE_FORMAT_OPTIONS}
          value={rows.imageFormat}
          onchange={(value) => updateGeneral('imageFormat', value)}
        />
      {/if}
      {#if qualityVisible}
        <RangeRow
          icon="gauge"
          title="Lossy quality"
          description="Used for JPEG and WebP"
          min={40}
          max={100}
          step={1}
          unit="%"
          value={rows.lossyQuality}
          onchange={(value) => updateGeneral('lossyQuality', value)}
        />
      {/if}
    </SettingsSection>
  {/if}

  {#if namingVisible}
    <SettingsSection
      title="File naming"
      description="Click a token to add it. Tokens are filled in at the moment of capture."
    >
      <Card>
        <div class="naming">
          <label class="pattern">
            <span class="pattern-label">Name pattern</span>
            <input
              bind:this={patternInput}
              type="text"
              spellcheck="false"
              value={rows.namePattern}
              oninput={(event) => updateGeneral('namePattern', event.currentTarget.value)}
            />
          </label>
          <div class="tokens" aria-label="Insert a token">
            {#each NAME_TOKENS as item}
              <button
                type="button"
                class="hv token"
                title={item.hint}
                onmousedown={(event) => event.preventDefault()}
                onclick={() => addToken(item.token)}
              >
                {item.token}
              </button>
            {/each}
          </div>
          <div class="preview">
            <span class="preview-label">Preview</span>
            <span class="preview-value">{preview}</span>
          </div>
        </div>
      </Card>
    </SettingsSection>
  {/if}

  {#if showWorkflow}
    <SettingsSection
      title="Default workflow · PrtSc"
      description="What happens after a capture. Each hotkey can run its own workflow — this is the one PrtSc and the tray use."
    >
      {#if stepsVisible}
        <StepList bind:items={steps} onchange={applySteps} />
      {/if}
      {#if linkFormatVisible}
        <SelectRow
          icon="code"
          title="Copied link format"
          description="What gets pasted when you press Ctrl+V"
          options={LINK_FORMAT_OPTIONS}
          value={rows.copiedLinkFormat}
          onchange={(value) => updateGeneral('copiedLinkFormat', value)}
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
            {workflowCount} workflows
          </Button>
        </div>
      {/if}
    </SettingsSection>
  {/if}
</SettingsPage>

<style>
  .version {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 16px;
    padding: 16px 20px;
  }

  .version-text {
    display: flex;
    flex: 1 1 200px;
    flex-direction: column;
    gap: 2px;
  }

  .version-name {
    font-size: 16px;
    font-weight: 600;
  }

  .version-number {
    font-size: 12px;
    color: var(--text3);
  }

  .naming {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 16px 20px;
  }

  .pattern {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .pattern-label {
    font-size: 13px;
  }

  .pattern input {
    height: 34px;
    padding: 0 10px;
    border: 1px solid var(--stroke);
    border-bottom: 2px solid var(--accent);
    border-radius: 4px;
    background: var(--layer);
    color: var(--text);
    font-size: 13px;
    font-family: var(--type-mono);
  }

  .tokens {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .token {
    height: 28px;
    padding: 0 10px;
    border: 1px solid var(--stroke);
    background: var(--layer);
    color: var(--text2);
    border-radius: 14px;
    font-size: 12px;
    font-family: var(--type-mono);
  }

  .preview {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 8px;
    padding: 10px 12px;
    background: var(--subtle);
    border-radius: 4px;
    font-size: 12px;
  }

  .preview-label {
    color: var(--text3);
  }

  .preview-value {
    color: var(--text);
    font-family: var(--type-mono);
    word-break: break-all;
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
