<script lang="ts">
  import Icon from './Icon.svelte';

  interface Props {
    message: string;
    onclose: () => void;
    duration?: number;
  }

  let { message, onclose, duration = 6000 }: Props = $props();

  $effect(() => {
    if (message === '') {
      return;
    }
    const timer = setTimeout(onclose, duration);
    return () => clearTimeout(timer);
  });
</script>

{#if message}
  <div class="toast" role="alert">
    <span class="text">{message}</span>
    <button type="button" class="dismiss hv" aria-label="Dismiss" onclick={onclose}>
      <Icon name="close" size={12} strokeWidth={1.8} />
    </button>
  </div>
{/if}

<style>
  .toast {
    position: fixed;
    right: 16px;
    bottom: 16px;
    z-index: 1000;
    display: flex;
    align-items: flex-start;
    gap: 12px;
    max-width: min(360px, calc(100vw - 32px));
    padding: 10px 10px 10px 14px;
    background: var(--menuLayer);
    border: 1px solid var(--stroke);
    border-left: 3px solid var(--err);
    border-radius: 6px;
    box-shadow: 0 8px 24px var(--shadow);
    color: var(--text);
    font-size: 13px;
    line-height: 1.4;
  }

  .text {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .dismiss {
    display: grid;
    place-items: center;
    flex: none;
    width: 22px;
    height: 22px;
    padding: 0;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: var(--text2);
    cursor: pointer;
  }
</style>
