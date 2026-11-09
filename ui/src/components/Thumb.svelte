<script lang="ts">
  interface Props {
    id: string | number;
    alt: string;
    duration?: string;
    tag?: string;
    small?: boolean;
  }

  let { id, alt, duration, tag, small = false }: Props = $props();
  let loaded = $state(false);
  const src = $derived(`http://thumb.localhost/${id}`);
</script>

<div class="thumb" class:loaded class:small>
  <img {src} {alt} loading="lazy" decoding="async" onload={() => (loaded = true)} />
  {#if duration}
    <span class="duration">{tag ? `${tag} · ${duration}` : duration}</span>
  {/if}
</div>

<style>
  .thumb {
    position: relative;
    width: 100%;
    aspect-ratio: 16 / 10;
    background: var(--subtle);
    border: 1px solid var(--stroke);
    border-radius: 4px;
    overflow: hidden;
  }

  img {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: cover;
    opacity: 0;
  }

  .loaded img {
    opacity: 1;
  }

  .duration {
    position: absolute;
    right: 6px;
    bottom: 6px;
    background: rgba(0, 0, 0, 0.72);
    color: #fff;
    font-size: 11px;
    padding: 1px 5px;
    border-radius: 3px;
  }

  .small .duration {
    font-size: 10px;
  }
</style>
