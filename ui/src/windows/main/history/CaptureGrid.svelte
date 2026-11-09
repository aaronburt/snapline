<script lang="ts">
  import Badge from '../../../components/Badge.svelte';
  import Thumb from '../../../components/Thumb.svelte';
  import type { Capture } from './data';
  import { badgeStatus, typeTag } from './data';

  interface Props {
    groups: { label: string; count: string; items: Capture[] }[];
    selectedId: number | null;
    onselect: (id: number) => void;
    onopen: (id: number) => void;
  }

  let { groups, selectedId, onselect, onopen }: Props = $props();
</script>

{#each groups as group (group.label)}
  <section class="group" aria-label={group.label}>
    <h2>{group.label} <span>· {group.count}</span></h2>
    <div class="grid" role="group" aria-label={group.label}>
      {#each group.items as capture (capture.id)}
        <button
          type="button"
          class="hv card"
          class:selected={capture.id === selectedId}
          aria-pressed={capture.id === selectedId}
          onclick={() => onselect(capture.id)}
          ondblclick={() => onopen(capture.id)}
        >
          <Thumb
            id={capture.id}
            alt={capture.name}
            duration={capture.dur}
            tag={capture.dur ? typeTag(capture.type) : undefined}
          />
          <div class="meta">
            <div class="name">{capture.name}</div>
            <div class="line">
              <span class="when">{capture.time} · {capture.size}</span>
              <Badge status={badgeStatus(capture.status)} />
            </div>
          </div>
        </button>
      {/each}
    </div>
  </section>
{/each}

<style>
  .group {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  h2 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
  }

  h2 span {
    font-weight: 400;
    color: var(--text3);
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(176px, 1fr));
    gap: 12px;
  }

  .card {
    background: var(--card);
    color: var(--text);
    border-radius: 6px;
    padding: 6px 6px 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    text-align: left;
    min-width: 0;
    border: 1px solid var(--stroke);
  }

  .card.selected {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent);
  }

  .meta {
    padding: 0 4px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }

  .name {
    font-size: 12px;
    font-weight: 500;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .line {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
    font-size: 11px;
    color: var(--text3);
  }

  .when {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
