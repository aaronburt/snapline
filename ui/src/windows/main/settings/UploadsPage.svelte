<script lang="ts">
  import { onMount } from 'svelte';
  import Button from '../../../components/Button.svelte';
  import Card from '../../../components/Card.svelte';
  import EmptyState from '../../../components/EmptyState.svelte';
  import Icon from '../../../components/Icon.svelte';
  import Popover from '../../../components/Popover.svelte';
  import SegmentedRow from '../../../components/SegmentedRow.svelte';
  import SelectRow from '../../../components/SelectRow.svelte';
  import ToggleRow from '../../../components/ToggleRow.svelte';
  import type {
    DestinationInput,
    DestinationKind,
    DestinationTestOutcome,
    DestinationView,
  } from '../../../lib/ipc/upload';
  import {
    addDestination,
    exportSxcu,
    listDestinations,
    removeDestination,
    saveDestination,
    testDestination,
  } from '../../../lib/ipc/upload';
  import SettingsPage from './SettingsPage.svelte';
  import SettingsSection from './SettingsSection.svelte';
  import DestinationCard from './destinations/DestinationCard.svelte';
  import DestinationForm from './destinations/DestinationForm.svelte';
  import DestinationSelectRow from './destinations/DestinationSelectRow.svelte';
  import { errorText } from './destinations/model';
  import {
    HISTORY_OPTIONS,
    PROXY_OPTIONS,
    RETRY_OPTIONS,
    TIMEOUT_OPTIONS,
    loadSettings,
    settings,
    updateUploads,
  } from './settings';

  interface Props {
    filter?: string;
  }

  let { filter = '' }: Props = $props();

  const rows = $derived($settings.uploads);
  let destinations = $state<DestinationView[]>([]);
  let destinationError = $state('');
  let activeView = $state<DestinationView | null>(null);
  let addOpen = $state(false);

  onMount(() => {
    void loadSettings();
    void refreshDestinations();
  });

  async function refreshDestinations(): Promise<void> {
    try {
      destinations = await listDestinations();
      destinationError = '';
    } catch (failure) {
      destinationError = errorText(failure);
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

  const imagesVisible = $derived(matches('Images'));
  const videosVisible = $derived(matches('Videos and GIFs'));
  const textVisible = $derived(matches('Text and code'));
  const filesVisible = $derived(matches('Other files'));
  const shortenerVisible = $derived(matches('Link shortener'));
  const showDefaults = $derived(
    matches('Default destinations', 'Where each kind of capture goes when “Upload” runs.') ||
      imagesVisible ||
      videosVisible ||
      textVisible ||
      filesVisible ||
      shortenerVisible,
  );

  const accountsVisible = $derived(
    matches('Accounts and uploaders') ||
      matches('Add destination') ||
      destinations.some(
        (destination) =>
          matches(destination.name) ||
          matches(destination.detail) ||
          matches(destination.status === 'configured' ? 'Configured' : 'Not set up'),
      ),
  );

  const stripVisible = $derived(
    matches(
      'Remove location and camera data',
      'Strip EXIF metadata before anything leaves your PC',
    ),
  );
  const clipboardVisible = $derived(
    matches(
      'Ask before uploading from the clipboard',
      'Avoid sending something you copied by mistake',
    ),
  );
  const retryVisible = $derived(matches('Retry failed uploads'));
  const timeoutVisible = $derived(matches('Upload timeout'));
  const historyVisible = $derived(
    matches('Keep upload history', 'Links and deletion links shown in History'),
  );
  const proxyVisible = $derived(matches('Proxy', 'http://host:port') || matches('Custom proxy'));
  const showBehaviour = $derived(
    matches('Upload behaviour') ||
      stripVisible ||
      clipboardVisible ||
      retryVisible ||
      timeoutVisible ||
      historyVisible ||
      proxyVisible,
  );

  interface DestinationOption {
    id: string;
    label: string;
  }

  const saveLocally: DestinationOption = { id: 'save_locally_only', label: 'Save locally only' };
  const none: DestinationOption = { id: 'none', label: 'None' };

  function accepts(kind: DestinationKind, target: string): boolean {
    if (target === 'images') {
      return kind !== 'gist';
    }
    if (target === 'text') {
      return kind === 'gist' || kind === 'custom';
    }
    return kind === 's3' || kind === 'sftp' || kind === 'custom';
  }

  function optionValue(raw: string): string {
    if (raw === 'save_locally_only' || raw === 'Save locally only') {
      return 'save_locally_only';
    }
    if (raw === 'none' || raw === 'None') {
      return 'none';
    }
    const destination = destinations.find((item) => item.id === raw || item.name === raw);
    return destination?.id ?? raw;
  }

  function fallbackLabel(raw: string): string {
    if (raw === 'save_locally_only') {
      return 'Save locally only';
    }
    if (raw === 'none') {
      return 'None';
    }
    const destination = destinations.find((item) => item.id === raw || item.name === raw);
    return destination?.name ?? raw;
  }

  function optionsFor(target: string, raw: string): DestinationOption[] {
    const base =
      target === 'shortener'
        ? [none]
        : [
            saveLocally,
            ...destinations
              .filter((destination) => destination.configured && accepts(destination.kind, target))
              .map((destination) => ({ id: destination.id, label: destination.name })),
          ];
    const value = optionValue(raw);
    if (base.some((option) => option.id === value)) {
      return base;
    }
    return [...base, { id: value, label: fallbackLabel(raw) }];
  }

  async function add(kind: DestinationKind): Promise<void> {
    addOpen = false;
    destinationError = '';
    try {
      const view = await addDestination(kind);
      destinations = [...destinations, view];
      activeView = view;
    } catch (failure) {
      destinationError = errorText(failure);
    }
  }

  function open(view: DestinationView): void {
    activeView = view;
    destinationError = '';
  }

  async function save(input: DestinationInput): Promise<DestinationView> {
    const updated = await saveDestination(input);
    destinations = destinations.map((destination) =>
      destination.id === updated.id ? updated : destination,
    );
    activeView = updated;
    return updated;
  }

  function test(id: string): Promise<DestinationTestOutcome> {
    return testDestination(id);
  }

  async function remove(id: string): Promise<void> {
    await removeDestination(id);
    destinations = destinations.filter((destination) => destination.id !== id);
    if (activeView?.id === id) {
      activeView = null;
    }
  }

  function exportUploader(id: string, includeHeaderValues: boolean): Promise<string | null> {
    return exportSxcu(id, includeHeaderValues);
  }

  function formTitle(view: DestinationView): string {
    if (view.kind === 'custom') {
      return view.name === 'Custom uploader' ? 'Custom uploader' : `Custom uploader · ${view.name}`;
    }
    return view.name;
  }
</script>

<SettingsPage title="Upload destinations">
  {#if showDefaults}
    <SettingsSection
      title="Default destinations"
      description="Where each kind of capture goes when “Upload” runs."
    >
      {#if imagesVisible}
        <DestinationSelectRow
          icon="image"
          title="Images"
          options={optionsFor('images', rows.images)}
          value={optionValue(rows.images)}
          onchange={(value) => updateUploads('images', value)}
        />
      {/if}
      {#if videosVisible}
        <DestinationSelectRow
          icon="film"
          title="Videos and GIFs"
          options={optionsFor('videos', rows.videos)}
          value={optionValue(rows.videos)}
          onchange={(value) => updateUploads('videos', value)}
        />
      {/if}
      {#if textVisible}
        <DestinationSelectRow
          icon="text"
          title="Text and code"
          options={optionsFor('text', rows.text)}
          value={optionValue(rows.text)}
          onchange={(value) => updateUploads('text', value)}
        />
      {/if}
      {#if filesVisible}
        <DestinationSelectRow
          icon="file"
          title="Other files"
          options={optionsFor('other', rows.otherFiles)}
          value={optionValue(rows.otherFiles)}
          onchange={(value) => updateUploads('otherFiles', value)}
        />
      {/if}
      {#if shortenerVisible}
        <DestinationSelectRow
          icon="short"
          title="Link shortener"
          options={optionsFor('shortener', rows.linkShortener)}
          value={optionValue(rows.linkShortener)}
          onchange={(value) => updateUploads('linkShortener', value)}
        />
      {/if}
    </SettingsSection>
  {/if}

  {#if accountsVisible}
    <section aria-label="Accounts and uploaders">
      <div class="accounts-head">
        <h2>Accounts and uploaders</h2>
        <div class="account-actions">
          <div class="add-wrap">
            <Button
              variant="accent"
              height={32}
              icon="plus"
              ariaHaspopup="dialog"
              ariaExpanded={addOpen}
              onclick={() => (addOpen = !addOpen)}
            >
              Add destination
            </Button>
            <Popover
              label="Add destination"
              align="end"
              bind:open={addOpen}
              onclose={() => (addOpen = false)}
            >
              <div class="menu">
                <button type="button" class="hv" onclick={() => add('imgur')}>
                  <Icon name="image" size={16} />
                  Imgur
                </button>
                <button type="button" class="hv" onclick={() => add('s3')}>
                  <Icon name="bucket" size={16} />
                  Amazon S3
                </button>
                <button type="button" class="hv" onclick={() => add('gist')}>
                  <Icon name="code" size={16} />
                  GitHub Gist
                </button>
                <button type="button" class="hv" onclick={() => add('sftp')}>
                  <Icon name="server" size={16} />
                  SFTP
                </button>
                <button type="button" class="hv" onclick={() => add('custom')}>
                  <Icon name="cloud" size={16} />
                  Custom uploader
                </button>
              </div>
            </Popover>
          </div>
        </div>
      </div>

      {#if destinations.length === 0}
        <Card>
          <div class="accounts-empty">
            <EmptyState title="No destinations yet" text="Add a destination to upload captures." />
          </div>
        </Card>
      {:else}
        <Card>
          {#each destinations as destination, index (destination.id)}
            <DestinationCard view={destination} first={index === 0} onaction={open} />
          {/each}
        </Card>
      {/if}

      {#if destinationError}
        <p class="error" role="alert">{destinationError}</p>
      {/if}

      {#if activeView}
        <Card>
          <div class="destination-form">
            <div class="form-head">
              <h3>{formTitle(activeView)}</h3>
              {#if activeView.kind === 'custom'}
                <p class="description">
                  Compatible with ShareX .sxcu files. Response values use JSON paths.
                </p>
              {/if}
            </div>
            <DestinationForm
              view={activeView}
              onsave={save}
              ontest={test}
              onremove={remove}
              onexport={exportUploader}
            />
          </div>
        </Card>
      {/if}
    </section>
  {/if}

  {#if showBehaviour}
    <SettingsSection title="Upload behaviour">
      {#if stripVisible}
        <ToggleRow
          icon="shield"
          title="Remove location and camera data"
          description="Strip EXIF metadata before anything leaves your PC"
          checked={rows.removeMetadata}
          onchange={(value) => updateUploads('removeMetadata', value)}
        />
      {/if}
      {#if clipboardVisible}
        <ToggleRow
          icon="clip"
          title="Ask before uploading from the clipboard"
          description="Avoid sending something you copied by mistake"
          checked={rows.askClipboard}
          onchange={(value) => updateUploads('askClipboard', value)}
        />
      {/if}
      {#if retryVisible}
        <SelectRow
          icon="redo"
          title="Retry failed uploads"
          options={RETRY_OPTIONS}
          value={rows.retry}
          onchange={(value) => updateUploads('retry', value)}
        />
      {/if}
      {#if timeoutVisible}
        <SegmentedRow
          icon="timer"
          title="Upload timeout"
          options={TIMEOUT_OPTIONS}
          value={rows.timeout}
          onchange={(value) => updateUploads('timeout', value)}
        />
      {/if}
      {#if historyVisible}
        <SelectRow
          icon="history"
          title="Keep upload history"
          description="Links and deletion links shown in History"
          options={HISTORY_OPTIONS}
          value={rows.keepHistory}
          onchange={(value) => updateUploads('keepHistory', value)}
        />
      {/if}
      {#if proxyVisible}
        <SelectRow
          icon="globe"
          title="Proxy"
          options={PROXY_OPTIONS}
          value={rows.proxy}
          onchange={(value) => updateUploads('proxy', value)}
        />
        {#if rows.proxy === 'Custom proxy…'}
          <div class="row">
            <Icon name="globe" size={20} />
            <div class="text">
              <span class="title">Custom proxy</span>
            </div>
            <input
              class="proxy-input"
              type="text"
              aria-label="Custom proxy"
              spellcheck="false"
              placeholder="http://host:port"
              value={rows.customProxy}
              oninput={(event) => updateUploads('customProxy', event.currentTarget.value)}
            />
          </div>
        {/if}
      {/if}
    </SettingsSection>
  {/if}
</SettingsPage>

<style>
  section {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .accounts-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin-bottom: 4px;
  }

  h2 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
  }

  h3 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
  }

  .account-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .add-wrap {
    position: relative;
  }

  .menu {
    display: flex;
    flex-direction: column;
    min-width: 180px;
    gap: 2px;
  }

  .menu button {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 34px;
    padding: 0 10px;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: var(--text);
    font-size: 13px;
    text-align: left;
    cursor: pointer;
  }

  .accounts-empty {
    padding: 28px 20px;
  }

  .destination-form {
    padding: 20px;
  }

  .form-head {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-bottom: 16px;
  }

  .description {
    margin: 0;
    font-size: 12px;
    color: var(--text3);
  }

  .error {
    margin: 0;
    color: var(--err);
    font-size: 12px;
  }

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

  .proxy-input {
    flex: 1 1 320px;
    max-width: 460px;
    min-width: 0;
    height: 34px;
    padding: 0 10px;
    border: 1px solid var(--stroke);
    border-bottom-color: var(--strokeHi);
    border-radius: 4px;
    background: var(--layer);
    color: var(--text);
    font-size: 12px;
    font-family: var(--type-mono);
  }
</style>
