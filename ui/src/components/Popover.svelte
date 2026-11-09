<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    open?: boolean;
    label: string;
    onclose?: () => void;
    children?: Snippet;
  }

  let { open = $bindable(false), label, onclose, children }: Props = $props();
  let element = $state<HTMLDivElement | null>(null);

  function close() {
    open = false;
    onclose?.();
  }

  function onPointer(event: PointerEvent) {
    if (open && element && !element.contains(event.target as Node)) {
      close();
    }
  }

  function onKey(event: KeyboardEvent) {
    if (open && event.key === 'Escape') {
      close();
    }
  }
</script>

<svelte:window onkeydown={onKey} onpointerdown={onPointer} />

{#if open}
  <div class="popover" role="dialog" aria-label={label} bind:this={element}>
    {@render children?.()}
  </div>
{/if}

<style>
  .popover {
    position: absolute;
    z-index: 20;
    background: var(--card);
    border: 1px solid var(--stroke);
    border-radius: 6px;
    box-shadow: 0 16px 40px var(--shadow);
    padding: 12px;
  }
</style>
