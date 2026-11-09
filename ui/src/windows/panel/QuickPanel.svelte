<script lang="ts">
  import { onMount, tick } from 'svelte';
  import Icon from '../../components/Icon.svelte';
  import Logo from '../../components/Logo.svelte';
  import Thumb from '../../components/Thumb.svelte';
  import { panelCaptures, panelLastLink, panelRecents } from './data';
  import type { PanelRecent } from './data';
  import {
    openCapture,
    openMain,
    panelAction,
    panelClose,
    panelCopyLink,
    panelResize,
  } from './ipc';

  interface Props {
    recents?: PanelRecent[];
    lastLink?: string | null;
  }

  let { recents = panelRecents, lastLink = panelLastLink }: Props = $props();

  let frame: HTMLElement | undefined = $state();

  const link = $derived(lastLink ? withoutScheme(lastLink) : null);

  function withoutScheme(value: string): string {
    if (value.startsWith('https://')) {
      return value.slice(8);
    }
    if (value.startsWith('http://')) {
      return value.slice(7);
    }
    return value;
  }

  async function reportHeight(): Promise<void> {
    await tick();
    if (!frame) {
      return;
    }
    const height = Math.ceil(frame.getBoundingClientRect().height);
    await panelResize(height).catch(() => undefined);
  }

  onMount(() => {
    void reportHeight();
  });

  function close(): void {
    void panelClose().catch(() => undefined);
  }

  function onKeydown(event: KeyboardEvent): void {
    if (event.key === 'Escape') {
      event.preventDefault();
      close();
    }
  }

  function runAction(action: string): void {
    void panelAction(action).catch(() => close());
  }

  function showRoute(route: string): void {
    void openMain(route).catch(() => close());
  }

  function showCapture(id: number): void {
    void openCapture(id).catch(() => close());
  }

  function copyLastLink(): void {
    if (!link) {
      return;
    }
    void panelCopyLink().catch(() => undefined);
  }
</script>

<svelte:window onkeydown={onKeydown} />

<main class="panel" aria-label="Snapline quick panel" bind:this={frame}>
  <header class="header">
    <Logo size={32} />
    <div class="title">Snapline</div>
    <button
      type="button"
      class="icon-button hv"
      aria-label="Open Snapline"
      onclick={() => showRoute('/history')}
    >
      <Icon name="open" size={18} />
    </button>
    <button
      type="button"
      class="icon-button hv"
      aria-label="Settings"
      onclick={() => showRoute('/settings/general')}
    >
      <Icon name="gearDetail" size={18} />
    </button>
  </header>

  <div class="tiles">
    {#each panelCaptures as capture (capture.id)}
      <button
        type="button"
        class="tile hv"
        class:accent={capture.accent === true}
        class:card={capture.accent !== true}
        title={`${capture.label} (${capture.chord})`}
        onclick={() => runAction(capture.action)}
      >
        <Icon name={capture.icon} size={20} />
        <span>{capture.label}</span>
      </button>
    {/each}
  </div>

  <section class="recent" aria-labelledby="recent-title">
    <div class="recent-header">
      <div class="recent-title" id="recent-title">Recent</div>
      <button type="button" class="view-all hv" onclick={() => showRoute('/history')}>
        View all
      </button>
    </div>
    {#if recents.length === 0}
      <div class="empty">No captures yet</div>
    {:else}
      <div class="recent-grid">
        {#each recents as recent (recent.id)}
          <button
            type="button"
            class="recent-item hv"
            aria-label={recent.name}
            onclick={() => showCapture(recent.id)}
          >
            <Thumb
              id={recent.id}
              alt={recent.name}
              duration={recent.duration}
              small
            />
            <span class="recent-meta">
              {#if recent.uploaded}
                <span class="uploaded" role="img" aria-label="Uploaded">
                  <Icon name="link" size={12} />
                </span>
              {/if}
              {recent.time}
            </span>
          </button>
        {/each}
      </div>
    {/if}
  </section>

  <footer class="footer">
    <span class="last-link-label">Last link</span>
    <span class="last-link">{link ?? 'None yet'}</span>
    <button
      type="button"
      class="copy hv"
      disabled={!link}
      onclick={copyLastLink}
    >
      <Icon name="copy" size={14} />
      Copy
    </button>
  </footer>
</main>

<style>
  .panel {
    width: 344px;
    background: var(--layer);
    border: 1px solid var(--stroke);
    border-radius: 8px;
    box-shadow: 0 16px 40px var(--shadow);
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }

  .header {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 14px 12px 12px 16px;
  }

  .title {
    flex: 1;
    min-width: 0;
    font-size: 14px;
    font-weight: 600;
  }

  .icon-button {
    width: 36px;
    height: 36px;
    border: none;
    border-radius: 6px;
    padding: 0;
    background: transparent;
    color: var(--text2);
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .tiles {
    padding: 0 12px 12px;
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 6px;
  }

  .tile {
    height: 56px;
    border-radius: 6px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 4px;
    padding: 0 4px;
    font-size: 12px;
  }

  .tile.accent {
    background: var(--accent);
    color: var(--onAccent);
    border: 1px solid var(--accent);
  }

  .tile.card {
    background: var(--card);
    color: var(--text);
    border: 1px solid var(--stroke);
  }

  .tile span {
    font-weight: 500;
    white-space: nowrap;
  }

  .recent {
    padding: 10px 16px 12px;
    border-top: 1px solid var(--stroke);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .recent-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .recent-title {
    font-size: 12px;
    color: var(--text3);
    font-weight: 500;
  }

  .view-all {
    font-size: 12px;
    color: var(--accent);
    padding: 4px 0;
    border: none;
    background: transparent;
  }

  .recent-grid {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 8px;
  }

  .recent-item {
    background: transparent;
    border: none;
    padding: 0;
    color: inherit;
    text-align: left;
    display: flex;
    flex-direction: column;
    gap: 5px;
    border-radius: 4px;
  }

  .recent-meta {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    color: var(--text3);
  }

  .uploaded {
    display: inline-flex;
    color: var(--ok);
  }

  .empty {
    font-size: 12px;
    color: var(--text3);
  }

  .footer {
    padding: 8px 8px 8px 16px;
    background: var(--mica);
    border-top: 1px solid var(--stroke);
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .last-link-label {
    font-size: 12px;
    color: var(--text3);
    flex: none;
  }

  .last-link {
    flex: 1;
    min-width: 0;
    font-size: 12px;
    color: var(--text2);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    font-family: var(--type-mono);
  }

  .copy {
    height: 32px;
    padding: 0 10px;
    border: 1px solid var(--stroke);
    background: var(--card);
    color: var(--text);
    border-radius: 4px;
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
  }

  .copy:disabled {
    cursor: default;
  }

  .copy:disabled:hover {
    box-shadow: none;
  }
</style>
