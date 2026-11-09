<script lang="ts">
  import Icon from '../../../../components/Icon.svelte';
  import type { DestinationView } from '../../../../lib/ipc/upload';
  import { destinationActionText, destinationIcon, destinationStatusText } from './model';

  interface Props {
    view: DestinationView;
    first?: boolean;
    onaction: (view: DestinationView) => void;
  }

  let { view, first = false, onaction }: Props = $props();
</script>

<div class="account" class:first>
  <span class="tile" aria-hidden="true">
    <Icon name={destinationIcon(view.kind)} size={18} />
  </span>
  <div class="text">
    <span class="name">{view.name}</span>
    <span class="detail">{view.detail}</span>
  </div>
  <span class="badge" class:ok={view.configured}>
    <span class="dot" class:ok={view.configured} aria-hidden="true"></span>
    {destinationStatusText(view.configured)}
  </span>
  <button type="button" class="hv action" onclick={() => onaction(view)}>
    {destinationActionText(view.action)}
  </button>
</div>

<style>
  .account {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 12px 16px;
    min-height: 64px;
    padding: 10px 20px;
    box-sizing: border-box;
  }

  .account {
    border-top: 1px solid var(--stroke);
  }

  .account.first {
    border-top: none;
  }

  .tile {
    width: 36px;
    height: 36px;
    flex: none;
    display: flex;
    align-items: center;
    justify-content: center;
    border: 1px solid var(--stroke);
    border-radius: 8px;
    background: var(--subtle);
    color: var(--text2);
  }

  .text {
    flex: 1 1 220px;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .name {
    font-size: 14px;
  }

  .detail {
    font-size: 12px;
    color: var(--text3);
  }

  .badge {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 2px 10px;
    border-radius: 12px;
    background: var(--subtle);
    color: var(--text2);
    font-size: 12px;
  }

  .badge.ok {
    background: var(--okSoft);
    color: var(--ok);
  }

  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--text3);
  }

  .dot.ok {
    background: var(--ok);
  }

  .action {
    height: 32px;
    min-width: 96px;
    padding: 0 12px;
    border: 1px solid var(--stroke);
    border-bottom-color: var(--strokeHi);
    border-radius: 4px;
    background: var(--layer);
    color: var(--text);
    font-size: 13px;
  }

  .action:not(:disabled) {
    cursor: pointer;
  }
</style>
