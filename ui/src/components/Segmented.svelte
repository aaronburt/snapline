<script lang="ts">
  interface Props {
    options: string[];
    value?: string;
    label?: string;
    labelledby?: string;
    onchange?: (value: string) => void;
  }

  let { options, value = $bindable(''), label, labelledby, onchange }: Props = $props();

  const active = $derived(options.includes(value) ? value : options[0]);

  function onkeydown(event: KeyboardEvent) {
    if (options.length === 0) {
      return;
    }
    if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) {
      return;
    }
    const current = Math.max(0, options.indexOf(active));
    let next = current;
    if (event.key === 'ArrowLeft' || event.key === 'ArrowUp') {
      next = (current + options.length - 1) % options.length;
    } else if (event.key === 'ArrowRight' || event.key === 'ArrowDown') {
      next = (current + 1) % options.length;
    } else if (event.key === 'Home') {
      next = 0;
    } else {
      next = options.length - 1;
    }
    event.preventDefault();
    value = options[next];
    onchange?.(value);
    const buttons = (event.currentTarget as HTMLElement).querySelectorAll<HTMLButtonElement>(
      '[role="radio"]',
    );
    buttons[next]?.focus();
  }
</script>

<div
  class="segmented"
  role="radiogroup"
  aria-label={label}
  aria-labelledby={labelledby}
  tabindex="-1"
  {onkeydown}
>
  {#each options as option}
    <button
      type="button"
      role="radio"
      aria-checked={value === option}
      tabindex={active === option ? 0 : -1}
      class:selected={value === option}
      onclick={() => {
        value = option;
        onchange?.(option);
      }}
    >
      {option}
    </button>
  {/each}
</div>

<style>
  .segmented {
    display: flex;
    padding: 2px;
    background: var(--subtle);
    border: 1px solid var(--stroke);
    border-radius: 6px;
  }

  button {
    height: 30px;
    min-width: 52px;
    padding: 0 12px;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: var(--text2);
    font-size: 12px;
    cursor: pointer;
  }

  button.selected {
    background: var(--card);
    color: var(--text);
    font-weight: 600;
    box-shadow: 0 1px 2px var(--shadow);
  }
</style>
