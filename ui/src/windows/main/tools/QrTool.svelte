<script lang="ts">
  import Segmented from '../../../components/Segmented.svelte';
  import Button from '../../../components/Button.svelte';
  import FieldRow from '../../../components/FieldRow.svelte';
  import Icon from '../../../components/Icon.svelte';
  import {
    qrCopyImage,
    qrCopyText,
    qrMake,
    qrOpen,
    qrSavePng,
    qrScan,
    type QrScanned,
  } from '../../../lib/ipc/tool';

  let tab = $state('Scan');
  let text = $state('');
  let svg = $state('');
  let makeError = $state('');
  let makeStatus = $state('');
  let scanned = $state<QrScanned | null>(null);
  let scanError = $state('');
  let scanStatus = $state('');
  let scanning = $state(false);
  let timer: ReturnType<typeof setTimeout> | undefined;

  function messageOf(error: unknown): string {
    if (typeof error === 'object' && error !== null && 'message' in error) {
      const message = (error as { message?: unknown }).message;
      if (typeof message === 'string' && message.length > 0) {
        return message;
      }
    }
    return 'The request could not be completed.';
  }

  async function refresh(value: string): Promise<void> {
    try {
      const made = await qrMake(value);
      if (value === text) {
        svg = made.svg;
        makeError = '';
      }
    } catch (error) {
      if (value === text) {
        svg = '';
        makeError = messageOf(error);
      }
    }
  }

  $effect(() => {
    const value = text;
    if (timer !== undefined) {
      clearTimeout(timer);
      timer = undefined;
    }
    makeStatus = '';
    if (value.trim() === '') {
      svg = '';
      makeError = '';
      return;
    }
    timer = setTimeout(() => {
      void refresh(value);
    }, 300);
    return () => {
      if (timer !== undefined) {
        clearTimeout(timer);
        timer = undefined;
      }
    };
  });

  async function copyImage(): Promise<void> {
    makeStatus = '';
    try {
      await qrCopyImage(text);
      makeStatus = 'Image copied';
    } catch (error) {
      makeStatus = messageOf(error);
    }
  }

  async function savePng(): Promise<void> {
    makeStatus = '';
    try {
      const path = await qrSavePng(text);
      makeStatus = `Saved to ${path}`;
    } catch (error) {
      makeStatus = messageOf(error);
    }
  }

  async function selectArea(): Promise<void> {
    scanning = true;
    scanError = '';
    scanStatus = '';
    try {
      scanned = await qrScan();
    } catch (error) {
      scanned = null;
      scanError = messageOf(error);
    } finally {
      scanning = false;
    }
  }

  async function copyText(): Promise<void> {
    if (!scanned) {
      return;
    }
    scanStatus = '';
    try {
      await qrCopyText(scanned.text);
      scanStatus = 'Text copied';
    } catch (error) {
      scanStatus = messageOf(error);
    }
  }

  async function openLink(): Promise<void> {
    if (!scanned?.link) {
      return;
    }
    scanStatus = '';
    try {
      await qrOpen(scanned.link);
      scanStatus = 'Opened in your browser';
    } catch (error) {
      scanStatus = messageOf(error);
    }
  }
</script>

<main class="page">
  <div class="pagehead">
    <div class="titles">
      <h1>QR code</h1>
      <div class="sub">Scan a QR code on screen, or make one from a link</div>
    </div>
  </div>

  <div class="panel">
    <Segmented options={['Scan', 'Make']} bind:value={tab} label="QR code mode" />

    {#if tab === 'Scan'}
      <section class="section" aria-label="Scan">
        <p class="hint">Select an area of the screen that contains a QR code.</p>
        <div class="actions">
          <Button variant="accent" icon="region" onclick={selectArea} disabled={scanning}>
            {scanning ? 'Selecting…' : 'Select area'}
          </Button>
        </div>
        {#if scanError}
          <p class="notice" role="status">{scanError}</p>
        {/if}
        {#if scanned}
          <pre class="result" aria-label="QR code text">{scanned.text}</pre>
          <div class="actions">
            <Button icon="copy" onclick={copyText}>Copy</Button>
            {#if scanned.link}
              <Button icon="open" onclick={openLink}>Open</Button>
            {/if}
          </div>
        {/if}
        {#if scanStatus}
          <p class="status" role="status">{scanStatus}</p>
        {/if}
      </section>
    {:else}
      <section class="section" aria-label="Make">
        <FieldRow
          icon="qr"
          title="Text or link"
          description="The QR code updates as you type."
          value={text}
          onchange={(value) => {
            text = value;
          }}
          placeholder="https://example.com"
        />
        <div class="makerow">
          <div class="preview">
            {#if svg}
              {@html svg}
            {:else}
              <span class="empty">
                <Icon name="qr" size={28} />
                <span>Type something to see the QR code.</span>
              </span>
            {/if}
          </div>
          <div class="side">
            <Button icon="copy" onclick={copyImage} disabled={!svg}>Copy image</Button>
            <Button icon="save" onclick={savePng} disabled={!svg}>Save as PNG</Button>
          </div>
        </div>
        {#if makeError}
          <p class="notice" role="status">{makeError}</p>
        {/if}
        {#if makeStatus}
          <p class="status" role="status">{makeStatus}</p>
        {/if}
      </section>
    {/if}
  </div>
</main>

<style>
  .page {
    flex: 999 1 460px;
    min-width: 0;
    padding: 20px 24px 32px;
    display: flex;
    flex-direction: column;
    gap: 20px;
    overflow-y: auto;
  }

  .pagehead {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    justify-content: space-between;
    gap: 12px;
  }

  .titles {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  h1 {
    margin: 0;
    font-size: 28px;
    line-height: 36px;
    font-weight: 600;
  }

  .sub {
    font-size: 12px;
    color: var(--text3);
  }

  .panel {
    width: 100%;
    max-width: 760px;
    padding: 20px;
    background: var(--card);
    border: 1px solid var(--stroke);
    border-radius: 6px;
    display: flex;
    flex-direction: column;
    gap: 20px;
  }

  .section {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .hint {
    margin: 0;
    font-size: 12px;
    line-height: 18px;
    color: var(--text3);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }

  .makerow {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-start;
    gap: 20px;
  }

  .preview {
    width: 232px;
    height: 232px;
    padding: 12px;
    background: #ffffff;
    border: 1px solid var(--stroke);
    border-radius: 6px;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .preview :global(svg) {
    width: 100%;
    height: 100%;
    display: block;
  }

  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 12px;
    color: var(--text3);
    font-size: 12px;
    line-height: 18px;
    text-align: center;
  }

  .side {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .result {
    margin: 0;
    min-height: 80px;
    max-height: 240px;
    padding: 12px;
    overflow: auto;
    background: var(--layer);
    border: 1px solid var(--stroke);
    border-radius: 4px;
    font-family: var(--type-mono);
    font-size: 12px;
    line-height: 18px;
    white-space: pre-wrap;
    word-break: break-word;
  }

  .notice {
    margin: 0;
    padding: 10px 12px;
    border: 1px solid var(--err);
    border-radius: 4px;
    color: var(--err);
    font-size: 12px;
  }

  .status {
    margin: 0;
    font-size: 12px;
    color: var(--text2);
  }
</style>
