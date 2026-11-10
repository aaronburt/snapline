<script lang="ts">
  interface Props {
    label?: string;
    sent?: number;
    total?: number;
    determinate?: boolean;
  }

  let { label = 'Upload progress', sent = 0, total = 0, determinate = false }: Props = $props();
  const known = $derived(determinate && total > 0);
</script>

<div class="track" class:indeterminate={!known}>
  <progress
    aria-label={label}
    value={known ? Math.min(Math.max(sent, 0), total) : undefined}
    max={known ? total : undefined}
  ></progress>
</div>

<style>
  .track {
    position: relative;
    width: 100%;
    min-width: 0;
    height: 4px;
    flex: 0 0 auto;
    overflow: hidden;
    border-radius: 2px;
    background: var(--strokeHi);
  }

  progress {
    appearance: none;
    display: block;
    width: 100%;
    height: 100%;
    padding: 0;
    border: 0;
    background: transparent;
  }

  progress::-webkit-progress-bar {
    background: transparent;
  }

  progress::-webkit-progress-value {
    background: var(--accent);
  }

  progress::-moz-progress-bar {
    background: var(--accent);
  }

  .indeterminate progress {
    opacity: 0;
  }

  .indeterminate::after {
    position: absolute;
    inset: 0 auto 0 0;
    width: 30%;
    background: var(--accent);
    content: '';
    animation: transfer 1.4s ease-in-out infinite;
  }

  @keyframes transfer {
    from { transform: translateX(-100%); }
    to { transform: translateX(335%); }
  }

  @media (prefers-reduced-motion: reduce) {
    .indeterminate::after {
      left: 35%;
      animation: none;
    }
  }
</style>
