<script lang="ts">
  import { onMount, tick } from 'svelte';
  import Logo from '../../../components/Logo.svelte';
  import Card from '../../../components/Card.svelte';
  import ToggleRow from '../../../components/ToggleRow.svelte';
  import SelectRow from '../../../components/SelectRow.svelte';
  import SegmentedRow from '../../../components/SegmentedRow.svelte';
  import RangeRow from '../../../components/RangeRow.svelte';
  import FieldRow from '../../../components/FieldRow.svelte';
  import DefaultWorkflowSection from './DefaultWorkflowSection.svelte';
  import SettingsPage from './SettingsPage.svelte';
  import SettingsSection from './SettingsSection.svelte';
  import {
    IMAGE_FORMAT_OPTIONS,
    NAME_TOKENS,
    SUBFOLDER_OPTIONS,
    imageFormatValue,
    loadSettings,
    pickFolder,
    requestPreview,
    settings,
    updateGeneral,
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
  const trayClickVisible = $derived(matches('Open capture on tray click', 'When off, clicking the tray icon opens the menu'));
  const showStartup = $derived(
    matches('Startup') || launchVisible || minimiseVisible || notificationsVisible || trayClickVisible,
  );

  const folderVisible = $derived(
    matches('Save captures to', 'Captures are kept on this PC even after they are uploaded'),
  );
  const subfoldersVisible = $derived(matches('Subfolders', 'Organise saved files automatically'));
  const formatVisible = $derived(
    matches('Image format', 'PNG is lossless; the others are smaller'),
  );
  const qualityVisible = $derived(matches('Lossy quality', 'Used for JPEG and WebP'));
  const showSaving = $derived(
    matches('Saving') || folderVisible || subfoldersVisible || formatVisible || qualityVisible,
  );

  const namingVisible = $derived(
    matches(
      'File naming',
      'Click a token to add it. Tokens are filled in at the moment of capture.',
    ) ||
      matches('Name pattern') ||
      matches('Preview'),
  );
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
      {#if trayClickVisible}
        <ToggleRow
          icon="tray"
          title="Open capture on tray click"
          description="When off, clicking the tray icon opens the menu"
          checked={rows.trayClickCapture}
          onchange={(value) => updateGeneral('trayClickCapture', value)}
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
          <div class="tokens" role="group" aria-label="Insert a token">
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
            <span class="preview-value" aria-live="polite">{preview}</span>
          </div>
        </div>
      </Card>
    </SettingsSection>
  {/if}

  <DefaultWorkflowSection {filter} />
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
</style>
