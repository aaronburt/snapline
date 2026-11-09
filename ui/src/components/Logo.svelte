<script lang="ts">
  interface Props {
    size?: number;
    dot?: boolean;
  }

  let { size = 32, dot = true }: Props = $props();
  const strokeWidth = $derived(size >= 32 ? 2.2 : 2.6);
</script>

<svg width={size} height={size} viewBox="0 0 32 32" aria-hidden="true">
  <rect width="32" height="32" rx="8" fill="var(--accent)"></rect>
  <path
    d="M9 13V9h4M19 9h4v4M23 19v4h-4M13 23H9v-4"
    fill="none"
    stroke="var(--onAccent)"
    stroke-width={strokeWidth}
    stroke-linecap="round"
    stroke-linejoin="round"
  ></path>
  {#if dot}
    <circle cx="16" cy="16" r="2.4" fill="var(--onAccent)"></circle>
  {/if}
</svg>
