<script lang="ts">
  import { onMount } from 'svelte';
  import Card from '../../../components/Card.svelte';
  import Button from '../../../components/Button.svelte';
  import Icon from '../../../components/Icon.svelte';
  import SelectRow from '../../../components/SelectRow.svelte';
  import SegmentedRow from '../../../components/SegmentedRow.svelte';
  import ToggleRow from '../../../components/ToggleRow.svelte';
  import EmptyState from '../../../components/EmptyState.svelte';
  import SettingsPage from './SettingsPage.svelte';
  import SettingsSection from './SettingsSection.svelte';
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
  const destinationOptions = ['Save locally only'];
  const shortenerOptions = ['None'];
  let activeForm = $state<string | null>(null);

  onMount(() => {
    void loadSettings();
  });

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
      matches('Import ShareX uploader (.sxcu)') ||
      matches('Add destination'),
  );

  const stripVisible = $derived(
    matches('Remove location and camera data', 'Strip EXIF metadata before anything leaves your PC'),
  );
  const clipboardVisible = $derived(
    matches('Ask before uploading from the clipboard', 'Avoid sending something you copied by mistake'),
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
</script>

<SettingsPage title="Upload destinations">
  {#if showDefaults}
    <SettingsSection
      title="Default destinations"
      description="Where each kind of capture goes when “Upload” runs."
    >
      {#if imagesVisible}
        <SelectRow
          icon="image"
          title="Images"
          options={destinationOptions}
          value={rows.images}
          onchange={(value) => updateUploads('images', value)}
        />
      {/if}
      {#if videosVisible}
        <SelectRow
          icon="film"
          title="Videos and GIFs"
          options={destinationOptions}
          value={rows.videos}
          onchange={(value) => updateUploads('videos', value)}
        />
      {/if}
      {#if textVisible}
        <SelectRow
          icon="text"
          title="Text and code"
          options={destinationOptions}
          value={rows.text}
          onchange={(value) => updateUploads('text', value)}
        />
      {/if}
      {#if filesVisible}
        <SelectRow
          icon="file"
          title="Other files"
          options={destinationOptions}
          value={rows.otherFiles}
          onchange={(value) => updateUploads('otherFiles', value)}
        />
      {/if}
      {#if shortenerVisible}
        <SelectRow
          icon="short"
          title="Link shortener"
          options={shortenerOptions}
          value={rows.linkShortener}
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
          <Button variant="secondary" height={32} icon="save">Import ShareX uploader (.sxcu)</Button>
          <Button variant="accent" height={32} icon="plus">Add destination</Button>
        </div>
      </div>
      <Card>
        <div class="accounts-empty">
          <EmptyState
            title="No destinations yet"
            text="Add a destination to upload captures."
          />
        </div>
      </Card>
      {#if activeForm}
        <Card>
          <div class="destination-form"></div>
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

  .account-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .accounts-empty {
    padding: 28px 20px;
  }

  .destination-form {
    padding: 20px;
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
