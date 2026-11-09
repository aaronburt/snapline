<script lang="ts">
  import type { Snippet } from 'svelte';
  import { onDestroy, tick } from 'svelte';

  interface Props {
    open?: boolean;
    label: string;
    align?: 'start' | 'end';
    onclose?: () => void;
    children?: Snippet;
  }

  let { open = $bindable(false), label, align = 'start', onclose, children }: Props = $props();
  let element = $state<HTMLDivElement | null>(null);
  let previous: HTMLElement | null = null;

  const selector =
    'button:not(:disabled), [href], input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex="-1"])';

  function focusable(): HTMLElement[] {
    if (!element) {
      return [];
    }
    return Array.from(element.querySelectorAll<HTMLElement>(selector));
  }

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
    if (!open) {
      return;
    }
    if (event.key === 'Escape') {
      event.preventDefault();
      close();
      return;
    }
    if (event.key !== 'Tab' || !element) {
      return;
    }
    const items = focusable();
    if (items.length === 0) {
      event.preventDefault();
      element.focus();
      return;
    }
    const first = items[0];
    const last = items[items.length - 1];
    const active = document.activeElement;
    if (event.shiftKey && (active === first || active === element)) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && active === last) {
      event.preventDefault();
      first.focus();
    }
  }

  $effect(() => {
    if (open) {
      previous = document.activeElement as HTMLElement | null;
      void tick().then(() => {
        const first = focusable()[0];
        if (first) {
          first.focus();
        } else {
          element?.focus();
        }
      });
    } else if (previous) {
      previous.focus();
      previous = null;
    }
  });

  onDestroy(() => {
    previous?.focus();
  });
</script>

<svelte:window onkeydown={onKey} onpointerdown={onPointer} />

{#if open}
  <div
    class="popover"
    class:end={align === 'end'}
    role="dialog"
    aria-modal="true"
    aria-label={label}
    tabindex="-1"
    bind:this={element}
  >
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

  .popover:focus-visible {
    outline: 2px solid var(--text);
    outline-offset: 1px;
  }

  .popover.end {
    right: 0;
  }
</style>
