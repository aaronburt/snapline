<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from '../../../components/Icon.svelte';
  import type { IconName } from '../../../lib/icons';
  import { editorOpenFile, toolAvailable } from '../../../lib/ipc/tool';

  interface Tool {
    name: string;
    icon: IconName;
    chord: string;
  }

  const tools: Tool[] = [
    { name: 'Colour picker', icon: 'colour', chord: 'Ctrl+Shift+C' },
    { name: 'Text capture (OCR)', icon: 'ocr', chord: 'Ctrl+Shift+T' },
    { name: 'Screen ruler', icon: 'ruler', chord: 'Ctrl+Shift+M' },
    { name: 'QR code', icon: 'qr', chord: '—' },
    { name: 'Image editor', icon: 'edit', chord: '—' },
    { name: 'Pin to screen', icon: 'pin', chord: '—' },
    { name: 'Image combiner', icon: 'width', chord: '—' },
    { name: 'Video converter', icon: 'film', chord: '—' },
  ];

  let editorAvailable = $state(false);
  let editorError = $state('');

  onMount(() => {
    void toolAvailable('tool_editor')
      .then((available) => {
        editorAvailable = available;
      })
      .catch(() => {
        editorAvailable = false;
      });
  });

  function describe(error: unknown): string {
    if (
      error !== null &&
      typeof error === 'object' &&
      'message' in error &&
      typeof (error as { message: unknown }).message === 'string'
    ) {
      return (error as { message: string }).message;
    }
    return 'The image editor could not be opened.';
  }

  function openEditor(): void {
    if (!editorAvailable) {
      return;
    }
    editorError = '';
    void editorOpenFile().catch((error: unknown) => {
      editorError = describe(error);
    });
  }
</script>

<main class="page">
  <div class="pagehead">
    <div class="titles">
      <h1>Tools</h1>
      <div class="sub">Utilities that don't need a capture first</div>
    </div>
  </div>

  {#if editorError}
    <p class="error" role="status">{editorError}</p>
  {/if}

  <div class="grid">
    {#each tools as tool (tool.name)}
      {#if tool.name === 'Image editor'}
        <button type="button" class="card" disabled={!editorAvailable} onclick={openEditor}>
          <span class="tile" aria-hidden="true">
            <Icon name={tool.icon} size={20} />
          </span>
          <span class="name">{tool.name}</span>
          <span class="desc">
            {editorAvailable ? 'Open any image to annotate, crop or blur.' : 'Coming in a later version'}
          </span>
          <span class="chord">{tool.chord}</span>
        </button>
      {:else}
        <button type="button" class="card" disabled>
          <span class="tile" aria-hidden="true">
            <Icon name={tool.icon} size={20} />
          </span>
          <span class="name">{tool.name}</span>
          <span class="desc">Coming in a later version</span>
          <span class="chord">{tool.chord}</span>
        </button>
      {/if}
    {/each}
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

  .error {
    margin: 0;
    font-size: 12px;
    color: var(--err);
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 12px;
  }

  .card {
    padding: 16px;
    background: var(--card);
    border: 1px solid var(--stroke);
    border-radius: 6px;
    color: var(--text);
    text-align: left;
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-height: 140px;
  }

  .card:disabled {
    opacity: 1;
  }

  .tile {
    width: 40px;
    height: 40px;
    border-radius: 8px;
    background: var(--accentSoft);
    color: var(--accent);
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .name {
    font-size: 14px;
    font-weight: 600;
  }

  .desc {
    font-size: 12px;
    line-height: 18px;
    color: var(--text3);
    flex: 1;
  }

  .chord {
    font-size: 11px;
    color: var(--text2);
    font-family: var(--type-mono);
  }
</style>
