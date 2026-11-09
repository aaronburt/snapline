<script lang="ts">
  import Badge from '../../../components/Badge.svelte';
  import Thumb from '../../../components/Thumb.svelte';
  import type { Capture } from './data';
  import { badgeStatus, typeLabel } from './data';

  interface Props {
    items: Capture[];
    selectedId: number | null;
    onselect: (id: number) => void;
    onopen: (id: number) => void;
  }

  let { items, selectedId, onselect, onopen }: Props = $props();
</script>

<div class="tablecard">
  <table>
    <thead>
      <tr>
        <th>File</th>
        <th>Type</th>
        <th>Size</th>
        <th>Captured</th>
        <th>Status</th>
      </tr>
    </thead>
    <tbody>
      {#each items as capture (capture.id)}
        <tr
          class:selected={capture.id === selectedId}
          aria-selected={capture.id === selectedId}
          tabindex="0"
          onclick={() => onselect(capture.id)}
          onkeydown={(event) => {
            if (event.key === 'Enter' || event.key === ' ') {
              event.preventDefault();
              onselect(capture.id);
            }
          }}
          ondblclick={() => onopen(capture.id)}
        >
          <td>
            <span class="file">
              <span class="thumb"><Thumb id={capture.id} alt={capture.name} /></span>
              <span class="filename">{capture.name}</span>
            </span>
          </td>
          <td class="muted">{typeLabel(capture.type)}</td>
          <td class="muted">{capture.size}</td>
          <td class="muted">{capture.day}, {capture.time}</td>
          <td><Badge status={badgeStatus(capture.status)} /></td>
        </tr>
      {/each}
    </tbody>
  </table>
</div>

<style>
  .tablecard {
    overflow-x: auto;
    background: var(--card);
    border: 1px solid var(--stroke);
    border-radius: 6px;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    min-width: 720px;
  }

  th {
    text-align: left;
    padding: 10px 16px;
    font-size: 12px;
    font-weight: 400;
    color: var(--text3);
    border-bottom: 1px solid var(--stroke);
  }

  td {
    height: 52px;
    padding: 0 16px;
    font-size: 12px;
    border-top: 1px solid var(--stroke);
  }

  thead tr:first-child th {
    border-top: none;
  }

  tbody tr {
    cursor: pointer;
  }

  tbody tr:hover {
    box-shadow: inset 0 0 0 999px var(--subtle);
  }

  tbody tr.selected {
    box-shadow: inset 0 0 0 1px var(--accent);
  }

  .file {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }

  .thumb {
    width: 40px;
    flex: none;
    display: block;
  }

  .filename {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .muted {
    color: var(--text2);
    white-space: nowrap;
  }
</style>
