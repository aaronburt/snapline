<script lang="ts">
  import { onMount } from 'svelte';
  import Button from '../../../components/Button.svelte';
  import FieldRow from '../../../components/FieldRow.svelte';
  import Icon from '../../../components/Icon.svelte';
  import RangeRow from '../../../components/RangeRow.svelte';
  import Segmented from '../../../components/Segmented.svelte';
  import Thumb from '../../../components/Thumb.svelte';
  import type { HistoryCaptureDto } from '../../../lib/ipc/bindings/HistoryCaptureDto';
  import { call } from '../../../lib/ipc/core';
  import { historyPage } from '../../../lib/ipc/history';

  type LayoutName = 'Side by side' | 'Stacked';
  type AlignName = 'Start' | 'Centre' | 'End';
  type BackgroundName = 'Transparent' | 'White' | 'Black' | 'Custom';

  type LayoutValue = 'sideBySide' | 'stacked';
  type AlignValue = 'start' | 'centre' | 'end';
  type BackgroundValue =
    | { kind: 'transparent' }
    | { kind: 'white' }
    | { kind: 'black' }
    | { kind: 'custom'; value: string };

  type CombineSource = { kind: 'history'; data: { id: number } } | { kind: 'picked'; data: { id: number } };

  interface CombineOptions {
    layout: LayoutValue;
    spacing: number;
    align: AlignValue;
    background: BackgroundValue;
  }

  type CombineRequest =
    | { kind: 'combinePickFiles' }
    | { kind: 'combinePreview'; data: { sources: CombineSource[]; options: CombineOptions } }
    | { kind: 'combineSave'; data: { sources: CombineSource[]; options: CombineOptions } };

  type CombineReply =
    | { kind: 'combinePickedFiles'; data: { files: Array<{ id: number; name: string }> } }
    | { kind: 'combinePreviewed'; data: { job: number } }
    | { kind: 'combineSaved'; data: { captureId: number } };

  interface SelectedSource {
    key: string;
    label: string;
    historyId: number | null;
    source: CombineSource;
  }

  let captures = $state<HistoryCaptureDto[]>([]);
  let historyLoading = $state(true);
  let historyError = $state('');
  let selected = $state<SelectedSource[]>([]);
  let layoutName = $state<LayoutName>('Side by side');
  let spacing = $state(8);
  let alignName = $state<AlignName>('Start');
  let backgroundName = $state<BackgroundName>('Transparent');
  let hex = $state('#ffffff');
  let previewJob = $state<number | null>(null);
  let previewLoading = $state(false);
  let previewError = $state('');
  let status = $state('');
  let saving = $state(false);
  let dragIndex: number | null = null;
  let previewGeneration = 0;

  function messageOf(error: unknown): string {
    if (typeof error === 'object' && error !== null && 'message' in error) {
      const message = (error as { message?: unknown }).message;
      if (typeof message === 'string' && message.length > 0) {
        return message;
      }
    }
    return 'The request could not be completed.';
  }

  function unexpected(): { code: string; message: string } {
    return {
      code: 'internal',
      message: 'Snapline returned an unexpected combiner answer.',
    };
  }

  function layoutValue(value: LayoutName): LayoutValue {
    return value === 'Stacked' ? 'stacked' : 'sideBySide';
  }

  function alignValue(value: AlignName): AlignValue {
    if (value === 'Centre') {
      return 'centre';
    }
    return value === 'End' ? 'end' : 'start';
  }

  function backgroundValue(): BackgroundValue {
    switch (backgroundName) {
      case 'White':
        return { kind: 'white' };
      case 'Black':
        return { kind: 'black' };
      case 'Custom':
        return { kind: 'custom', value: hex.trim() };
      default:
        return { kind: 'transparent' };
    }
  }

  function options(): CombineOptions {
    return {
      layout: layoutValue(layoutName),
      spacing,
      align: alignValue(alignName),
      background: backgroundValue(),
    };
  }

  function sources(): CombineSource[] {
    return selected.map((item) => item.source);
  }

  function orderOf(id: number): number {
    return selected.findIndex((item) => item.historyId === id);
  }

  function addHistory(capture: HistoryCaptureDto): void {
    const existing = orderOf(capture.id);
    if (existing >= 0) {
      remove(existing);
      return;
    }
    selected = [
      ...selected,
      {
        key: `history-${capture.id}`,
        label: capture.name,
        historyId: capture.id,
        source: { kind: 'history', data: { id: capture.id } },
      },
    ];
  }

  function remove(index: number): void {
    selected = selected.filter((_, position) => position !== index);
  }

  function move(from: number, to: number): void {
    if (from === to || to < 0 || to >= selected.length) {
      return;
    }
    const next = selected.slice();
    const [item] = next.splice(from, 1);
    next.splice(to, 0, item);
    selected = next;
  }

  function startDrag(event: DragEvent, index: number): void {
    dragIndex = index;
    event.dataTransfer?.setData('text/plain', String(index));
  }

  function allowDrop(event: DragEvent): void {
    event.preventDefault();
  }

  function dropOn(event: DragEvent, index: number): void {
    event.preventDefault();
    if (dragIndex !== null) {
      move(dragIndex, index);
    }
    dragIndex = null;
  }

  function endDrag(): void {
    dragIndex = null;
  }

  async function loadHistory(): Promise<void> {
    historyLoading = true;
    historyError = '';
    try {
      const page = await historyPage('images', null);
      captures = page.items;
    } catch (error) {
      historyError = messageOf(error);
    } finally {
      historyLoading = false;
    }
  }

  async function addFiles(): Promise<void> {
    historyError = '';
    try {
      const reply = await call<CombineRequest, CombineReply>('ipc_tool', {
        kind: 'combinePickFiles',
      });
      if (reply.kind !== 'combinePickedFiles') {
        throw unexpected();
      }
      const additions = reply.data.files.map((file) => ({
        key: `picked-${file.id}`,
        label: file.name,
        historyId: null,
        source: { kind: 'picked', data: { id: file.id } } as CombineSource,
      }));
      selected = [...selected, ...additions];
    } catch (error) {
      historyError = messageOf(error);
    }
  }

  async function loadPreview(optionsValue: CombineOptions, sourcesValue: CombineSource[], generation: number): Promise<void> {
    previewLoading = true;
    try {
      const reply = await call<CombineRequest, CombineReply>('ipc_tool', {
        kind: 'combinePreview',
        data: { sources: sourcesValue, options: optionsValue },
      });
      if (generation !== previewGeneration) {
        return;
      }
      if (reply.kind !== 'combinePreviewed') {
        throw unexpected();
      }
      previewJob = reply.data.job;
      previewError = '';
    } catch (error) {
      if (generation === previewGeneration) {
        previewJob = null;
        previewError = messageOf(error);
      }
    } finally {
      if (generation === previewGeneration) {
        previewLoading = false;
      }
    }
  }

  async function save(): Promise<void> {
    if (selected.length === 0) {
      return;
    }
    saving = true;
    status = '';
    try {
      const reply = await call<CombineRequest, CombineReply>('ipc_tool', {
        kind: 'combineSave',
        data: { sources: sources(), options: options() },
      });
      if (reply.kind !== 'combineSaved') {
        throw unexpected();
      }
      status = 'Saved to History';
    } catch (error) {
      status = messageOf(error);
    } finally {
      saving = false;
    }
  }

  $effect(() => {
    const generation = ++previewGeneration;
    const sourcesValue = selected.map((item) => item.source);
    const optionsValue = options();
    status = '';
    if (sourcesValue.length === 0) {
      previewJob = null;
      previewError = '';
      previewLoading = false;
      return;
    }
    const timer = setTimeout(() => {
      void loadPreview(optionsValue, sourcesValue, generation);
    }, 180);
    return () => {
      clearTimeout(timer);
    };
  });

  onMount(() => {
    void loadHistory();
  });
</script>

<main class="page">
  <div class="pagehead">
    <div class="titles">
      <h1>Image combiner</h1>
      <div class="sub">Join several captures side by side or stacked</div>
    </div>
  </div>

  <div class="workspace">
    <section class="panel" aria-label="Choose images">
      <div class="panelhead">
        <h2>Images</h2>
        <Button icon="plus" onclick={addFiles}>Add files…</Button>
      </div>
      <p class="hint">Pick captures from History, or add image files from your PC.</p>
      {#if historyError}
        <p class="notice" role="status">{historyError}</p>
      {/if}
      {#if historyLoading}
        <p class="hint">Loading captures…</p>
      {:else if captures.length === 0}
        <p class="hint">No image captures yet.</p>
      {:else}
        <div class="picks">
          {#each captures as capture (capture.id)}
            {@const order = orderOf(capture.id)}
            <button
              type="button"
              class="pick"
              class:selected={order >= 0}
              aria-pressed={order >= 0}
              onclick={() => addHistory(capture)}
            >
              <Thumb id={capture.id} alt={capture.name} small />
              <span class="pick-name">{capture.name}</span>
              {#if order >= 0}
                <span class="pick-order">{order + 1}</span>
              {/if}
            </button>
          {/each}
        </div>
      {/if}
    </section>

    <section class="panel" aria-label="Order">
      <div class="panelhead">
        <h2>Order</h2>
        {#if selected.length > 0}
          <Button variant="text" onclick={() => (selected = [])}>Clear</Button>
        {/if}
      </div>
      {#if selected.length === 0}
        <p class="hint">Choose at least one image. Drag the rows to change their order.</p>
      {:else}
        <ol class="orderlist">
          {#each selected as item, index (item.key)}
            <li
              draggable="true"
              ondragstart={(event) => startDrag(event, index)}
              ondragover={allowDrop}
              ondrop={(event) => dropOn(event, index)}
              ondragend={endDrag}
            >
              <span class="grip" aria-hidden="true"><Icon name="drag" size={14} /></span>
              {#if item.historyId !== null}
                <span class="mini"><Thumb id={item.historyId} alt={item.label} small /></span>
              {:else}
                <span class="mini file" aria-hidden="true"><Icon name="file" size={16} /></span>
              {/if}
              <span class="order-name">{item.label}</span>
              <span class="order-actions">
                <button
                  type="button"
                  class="icon"
                  aria-label={`Move ${item.label} up`}
                  title={index === 0 ? 'Already first.' : undefined}
                  disabled={index === 0}
                  onclick={() => move(index, index - 1)}>↑</button
                >
                <button
                  type="button"
                  class="icon"
                  aria-label={`Move ${item.label} down`}
                  title={index === selected.length - 1 ? 'Already last.' : undefined}
                  disabled={index === selected.length - 1}
                  onclick={() => move(index, index + 1)}>↓</button
                >
                <button
                  type="button"
                  class="icon"
                  aria-label={`Remove ${item.label}`}
                  onclick={() => remove(index)}
                >
                  <Icon name="close" size={12} />
                </button>
              </span>
            </li>
          {/each}
        </ol>
      {/if}
    </section>

    <section class="panel" aria-label="Layout">
      <h2>Layout</h2>
      <div class="control">
        <span class="label">Direction</span>
        <Segmented options={['Side by side', 'Stacked']} bind:value={layoutName} label="Direction" />
      </div>
      <div class="control">
        <span class="label">Align</span>
        <Segmented options={['Start', 'Centre', 'End']} bind:value={alignName} label="Align" />
      </div>
      <div class="control">
        <span class="label">Background</span>
        <Segmented
          options={['Transparent', 'White', 'Black', 'Custom']}
          bind:value={backgroundName}
          label="Background"
        />
      </div>
      {#if backgroundName === 'Custom'}
        <FieldRow
          icon="palette"
          title="Custom colour"
          description="A hex colour, for example #1A2B3C."
          value={hex}
          placeholder="#1A2B3C"
          onchange={(value) => {
            hex = value;
          }}
        />
      {/if}
      <RangeRow
        icon="width"
        title="Spacing"
        description="The gap between each image."
        min={0}
        max={64}
        unit=" px"
        bind:value={spacing}
      />
    </section>

    <section class="panel" aria-label="Preview">
      <div class="panelhead">
        <h2>Preview</h2>
        {#if previewLoading}
          <span class="hint">Updating…</span>
        {/if}
      </div>
      <div class="preview">
        {#if previewJob !== null}
          <img src={`http://combine.localhost/${previewJob}`} alt="Combined preview" />
        {:else}
          <span class="empty">
            <Icon name="combine" size={28} />
            <span>Add images to see the combined result.</span>
          </span>
        {/if}
      </div>
      {#if previewError}
        <p class="notice" role="status">{previewError}</p>
      {/if}
      <div class="footer">
        {#if status}
          <span class="status" role="status">{status}</span>
        {/if}
        <Button
          variant="accent"
          icon="save"
          onclick={save}
          disabled={selected.length === 0 || saving}
          title={selected.length === 0 ? 'Choose at least one image.' : undefined}
        >
          {saving ? 'Saving…' : 'Save'}
        </Button>
      </div>
    </section>
  </div>
</main>

<style>
  .page {
    flex: 999 1 460px;
    min-width: 0;
    padding: 20px 24px 32px;
    display: flex;
    flex-direction: column;
    gap: 20px;
    overflow-y: auto;
  }

  .pagehead {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    justify-content: space-between;
    gap: 12px;
  }

  .titles {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  h1 {
    margin: 0;
    font-size: 28px;
    line-height: 36px;
    font-weight: 600;
  }

  h2 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
  }

  .sub {
    font-size: 12px;
    color: var(--text3);
  }

  .workspace {
    width: 100%;
    max-width: 1200px;
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    gap: 16px;
    align-items: start;
  }

  .panel {
    min-width: 0;
    padding: 18px;
    background: var(--card);
    border: 1px solid var(--stroke);
    border-radius: 6px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .panelhead {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .hint {
    margin: 0;
    font-size: 12px;
    line-height: 18px;
    color: var(--text3);
  }

  .picks {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(104px, 1fr));
    gap: 8px;
  }

  .pick {
    position: relative;
    padding: 5px 5px 7px;
    background: var(--layer);
    border: 1px solid var(--stroke);
    border-radius: 6px;
    color: var(--text);
    text-align: left;
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
    cursor: pointer;
  }

  .pick.selected {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent);
  }

  .pick-name {
    font-size: 11px;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .pick-order {
    position: absolute;
    top: 8px;
    right: 8px;
    width: 18px;
    height: 18px;
    border-radius: 9px;
    background: var(--accent);
    color: var(--onAccent);
    font-size: 11px;
    font-weight: 600;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .orderlist {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .orderlist li {
    min-height: 44px;
    padding: 5px 6px;
    background: var(--layer);
    border: 1px solid var(--stroke);
    border-radius: 4px;
    display: flex;
    align-items: center;
    gap: 8px;
    cursor: grab;
  }

  .grip {
    color: var(--text3);
    display: flex;
  }

  .mini {
    width: 52px;
    flex: none;
  }

  .mini.file {
    height: 32px;
    border-radius: 4px;
    background: var(--subtle);
    border: 1px solid var(--stroke);
    color: var(--text3);
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .order-name {
    flex: 1;
    min-width: 0;
    font-size: 12px;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .order-actions {
    display: flex;
    align-items: center;
    gap: 2px;
  }

  .icon {
    width: 26px;
    height: 26px;
    padding: 0;
    border: 1px solid transparent;
    border-radius: 4px;
    background: transparent;
    color: var(--text2);
    font-size: 13px;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
  }

  .icon:hover:not(:disabled) {
    background: var(--subtle);
    border-color: var(--stroke);
  }

  .icon:disabled {
    opacity: 0.35;
    cursor: default;
  }

  .control {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 8px 12px;
  }

  .label {
    font-size: 13px;
    color: var(--text2);
  }

  .preview {
    min-height: 220px;
    padding: 12px;
    border: 1px solid var(--stroke);
    border-radius: 4px;
    background-color: var(--layer);
    background-image:
      linear-gradient(45deg, var(--subtle) 25%, transparent 25%),
      linear-gradient(-45deg, var(--subtle) 25%, transparent 25%),
      linear-gradient(45deg, transparent 75%, var(--subtle) 75%),
      linear-gradient(-45deg, transparent 75%, var(--subtle) 75%);
    background-size: 16px 16px;
    background-position:
      0 0,
      0 8px,
      8px -8px,
      -8px 0;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .preview img {
    display: block;
    max-width: 100%;
    max-height: 420px;
    object-fit: contain;
  }

  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 12px;
    color: var(--text3);
    font-size: 12px;
    line-height: 18px;
    text-align: center;
  }

  .footer {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 12px;
    min-height: 34px;
  }

  .status {
    margin-right: auto;
    font-size: 12px;
    color: var(--text2);
  }

  .notice {
    margin: 0;
    padding: 10px 12px;
    border: 1px solid var(--err);
    border-radius: 4px;
    color: var(--err);
    font-size: 12px;
  }

  @media (max-width: 900px) {
    .workspace {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
