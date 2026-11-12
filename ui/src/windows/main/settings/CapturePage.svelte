<script lang="ts">
  import { onMount } from 'svelte';
  import { monitors as getMonitors, type MonitorLayout } from '../../../lib/ipc/app';
  import Card from '../../../components/Card.svelte';
  import ToggleRow from '../../../components/ToggleRow.svelte';
  import SelectRow from '../../../components/SelectRow.svelte';
  import SegmentedRow from '../../../components/SegmentedRow.svelte';
  import RangeRow from '../../../components/RangeRow.svelte';
  import FieldRow from '../../../components/FieldRow.svelte';
  import SettingsPage from './SettingsPage.svelte';
  import SettingsSection from './SettingsSection.svelte';
  import {
    CAPTURE_DELAY_OPTIONS,
    GIF_FRAME_RATE_OPTIONS,
    GIF_WIDTH_OPTIONS,
    MAGNIFIER_ZOOM_OPTIONS,
    MICROPHONE_OPTIONS,
    MONITOR_OPTIONS,
    QUALITY_OPTIONS,
    RECORDING_FRAME_RATE_OPTIONS,
    VIDEO_FORMAT_OPTIONS,
    loadSettings,
    pickFfmpegPath,
    resetFfmpegPath,
    settings,
    updateCapture,
  } from './settings';

  interface Props {
    filter?: string;
  }

  let { filter = '' }: Props = $props();

  const rows = $derived($settings.capture);
  let displays = $state<MonitorLayout[] | null>(null);

  async function refreshMonitors() {
    try {
      displays = (await getMonitors()).filter((monitor) => monitor.width > 0 && monitor.height > 0);
    } catch {
      displays = [];
    }
  }

  onMount(() => {
    void loadSettings();
    void refreshMonitors();
  });

  function matches(title: string, description = ''): boolean {
    const needle = filter.trim().toLowerCase();
    return (
      needle === '' ||
      title.toLowerCase().includes(needle) ||
      description.toLowerCase().includes(needle)
    );
  }

  const cursorVisible = $derived(
    matches('Include mouse cursor', 'Draw the pointer into screenshots'),
  );
  const delayVisible = $derived(
    matches('Capture delay', 'Wait before capturing, so you can open a menu first'),
  );
  const shutterVisible = $derived(matches('Shutter sound', 'Play a sound when a capture is taken'));
  const shadowVisible = $derived(
    matches(
      'Keep window shadow and rounded corners',
      'Window captures keep the Windows 11 shadow on a transparent background',
    ),
  );
  const showScreenshots = $derived(
    matches('Screenshots') || cursorVisible || delayVisible || shutterVisible || shadowVisible,
  );

  const freezeVisible = $derived(matches('Freeze the screen while selecting'));
  const magnifierVisible = $derived(matches('Magnifier', 'Pixel-accurate zoom next to the cursor'));
  const zoomVisible = $derived(matches('Magnifier zoom'));
  const snapVisible = $derived(
    matches(
      'Snap to windows and controls',
      'Hover to pick a whole window, panel or button, then click',
    ),
  );
  const sizeVisible = $derived(
    matches('Show size and position', 'Width, height and coordinates while dragging'),
  );
  const crosshairVisible = $derived(
    matches('Full-screen crosshair', 'Guide lines across every monitor'),
  );
  const showRegion = $derived(
    matches('Region selection') ||
      freezeVisible ||
      magnifierVisible ||
      zoomVisible ||
      snapVisible ||
      sizeVisible ||
      crosshairVisible,
  );

  const monitorsVisible = $derived(
    matches('Monitors', 'Choose what a full-screen capture (Ctrl+PrtSc) includes.') ||
      matches('Full-screen capture uses'),
  );

  const videoFormatVisible = $derived(
    matches('Video format', 'H.264 plays everywhere; H.265 is smaller'),
  );
  const frameRateVisible = $derived(matches('Frame rate', 'Frames per second'));
  const qualityVisible = $derived(matches('Quality'));
  const systemAudioVisible = $derived(matches('Record system audio', 'Sound playing on this PC'));
  const microphoneVisible = $derived(matches('Record microphone'));
  const microphoneDeviceVisible = $derived(matches('Microphone device'));
  const borderVisible = $derived(
    matches('Show recording border', 'Outline the recorded area (not included in the video)'),
  );
  const ffmpegVisible = $derived(
    matches('FFmpeg location', 'Used to encode video. Found automatically.'),
  );
  const showRecording = $derived(
    matches('Screen recording') ||
      videoFormatVisible ||
      frameRateVisible ||
      qualityVisible ||
      systemAudioVisible ||
      microphoneVisible ||
      microphoneDeviceVisible ||
      borderVisible ||
      ffmpegVisible,
  );

  const gifFrameRateVisible = $derived(
    matches('Frame rate', 'Lower frame rates make much smaller GIFs'),
  );
  const gifWidthVisible = $derived(matches('Maximum width'));
  const paletteVisible = $derived(
    matches('Optimise colour palette', 'Smaller files, slightly slower to encode'),
  );
  const showGif = $derived(
    matches('GIF') || gifFrameRateVisible || gifWidthVisible || paletteVisible,
  );

  const topology = $derived.by(() => {
    if (!displays?.length) return null;
    const left = Math.min(...displays.map((monitor) => monitor.x));
    const top = Math.min(...displays.map((monitor) => monitor.y));
    const width = Math.max(...displays.map((monitor) => monitor.x + monitor.width)) - left;
    const height = Math.max(...displays.map((monitor) => monitor.y + monitor.height)) - top;
    const monitors = displays.map((monitor, index) => ({
      ...monitor,
      number: /DISPLAY(\d+)$/i.exec(monitor.name)?.[1] ?? String(index + 1),
      left: ((monitor.x - left) / width) * 100,
      top: ((monitor.y - top) / height) * 100,
      widthPercent: (monitor.width / width) * 100,
      heightPercent: (monitor.height / height) * 100,
    }));
    return { width, height, monitors, primary: monitors.find((monitor) => monitor.primary) };
  });

  const monitorCaption = $derived(
    topology &&
      (rows.monitorMode === 'All monitors'
        ? `All monitors are captured as one ${topology.width} × ${topology.height} image`
        : rows.monitorMode === 'Primary monitor'
          ? topology.primary
            ? `Monitor ${topology.primary.number} (primary) is captured`
            : 'The primary monitor is captured'
          : 'The monitor with the cursor is captured'),
  );

  const monitorLabel = $derived(
    topology?.monitors
      .map(
        (monitor) =>
          `Monitor ${monitor.number}, ${monitor.width} by ${monitor.height}${monitor.primary ? ', primary' : ''}`,
      )
      .join('; '),
  );

  const microphoneOptions = $derived(
    MICROPHONE_OPTIONS.includes(rows.microphoneDevice)
      ? MICROPHONE_OPTIONS
      : [...MICROPHONE_OPTIONS, rows.microphoneDevice],
  );
</script>

<svelte:window onfocus={refreshMonitors} />

<SettingsPage title="Capture">
  {#if showScreenshots}
    <SettingsSection title="Screenshots">
      {#if cursorVisible}
        <ToggleRow
          icon="cursor"
          title="Include mouse cursor"
          description="Draw the pointer into screenshots"
          checked={rows.includeCursor}
          onchange={(value) => updateCapture('includeCursor', value)}
        />
      {/if}
      {#if delayVisible}
        <SegmentedRow
          icon="timer"
          title="Capture delay"
          description="Wait before capturing, so you can open a menu first"
          options={CAPTURE_DELAY_OPTIONS}
          value={rows.captureDelay}
          onchange={(value) => updateCapture('captureDelay', value)}
        />
      {/if}
      {#if shutterVisible}
        <ToggleRow
          icon="speaker"
          title="Shutter sound"
          description="Play a sound when a capture is taken"
          checked={rows.shutterSound}
          onchange={(value) => updateCapture('shutterSound', value)}
        />
      {/if}
      {#if shadowVisible}
        <ToggleRow
          icon="shadow"
          title="Keep window shadow and rounded corners"
          description="Window captures keep the Windows 11 shadow on a transparent background"
          checked={rows.windowShadow}
          onchange={(value) => updateCapture('windowShadow', value)}
        />
      {/if}
    </SettingsSection>
  {/if}

  {#if showRegion}
    <SettingsSection title="Region selection">
      {#if freezeVisible}
        <ToggleRow
          icon="freeze"
          title="Freeze the screen while selecting"
          checked={rows.freezeScreen}
          onchange={(value) => updateCapture('freezeScreen', value)}
        />
      {/if}
      {#if magnifierVisible}
        <ToggleRow
          icon="searchPlus"
          title="Magnifier"
          description="Pixel-accurate zoom next to the cursor"
          checked={rows.magnifier}
          onchange={(value) => updateCapture('magnifier', value)}
        />
      {/if}
      {#if zoomVisible}
        <SegmentedRow
          icon="search"
          title="Magnifier zoom"
          options={MAGNIFIER_ZOOM_OPTIONS}
          value={rows.magnifierZoom}
          onchange={(value) => updateCapture('magnifierZoom', value)}
        />
      {/if}
      {#if snapVisible}
        <ToggleRow
          icon="snap"
          title="Snap to windows and controls"
          description="Hover to pick a whole window, panel or button, then click"
          checked={rows.snapWindows}
          onchange={(value) => updateCapture('snapWindows', value)}
        />
      {/if}
      {#if sizeVisible}
        <ToggleRow
          icon="ruler"
          title="Show size and position"
          description="Width, height and coordinates while dragging"
          checked={rows.showSize}
          onchange={(value) => updateCapture('showSize', value)}
        />
      {/if}
      {#if crosshairVisible}
        <ToggleRow
          icon="cross"
          title="Full-screen crosshair"
          description="Guide lines across every monitor"
          checked={rows.crosshair}
          onchange={(value) => updateCapture('crosshair', value)}
        />
      {/if}
    </SettingsSection>
  {/if}

  {#if monitorsVisible}
    <SettingsSection
      title="Monitors"
      description="Choose what a full-screen capture (Ctrl+PrtSc) includes."
    >
      <Card>
        <div class="monitor-card">
          {#if topology}
            <div
              class="monitor-layout"
              role="img"
              aria-label={monitorLabel}
              style:aspect-ratio={`${topology.width} / ${topology.height}`}
              style:max-width={`${Math.min(560, (220 * topology.width) / topology.height)}px`}
            >
              {#each topology.monitors as monitor (monitor.id)}
                <div
                  class="monitor-position"
                  style:left="{monitor.left}%"
                  style:top="{monitor.top}%"
                  style:width="{monitor.widthPercent}%"
                  style:height="{monitor.heightPercent}%"
                >
                  <div
                    class="monitor-box"
                    class:on={rows.monitorMode === 'All monitors' ||
                      (rows.monitorMode === 'Primary monitor' && monitor.primary)}
                  >
                    <span class="monitor-number">{monitor.number}</span>
                    <span class="monitor-resolution">
                      {monitor.width} × {monitor.height}{monitor.primary ? ' · primary' : ''}
                    </span>
                  </div>
                </div>
              {/each}
            </div>
          {/if}
          <div class="monitor-caption">
            {monitorCaption ??
              (displays === null ? 'Reading monitor layout…' : 'Monitor layout is unavailable.')}
          </div>
        </div>
      </Card>
      <SelectRow
        icon="screen"
        title="Full-screen capture uses"
        options={MONITOR_OPTIONS}
        value={rows.monitorMode}
        onchange={(value) => updateCapture('monitorMode', value)}
      />
    </SettingsSection>
  {/if}

  {#if showRecording}
    <SettingsSection title="Screen recording">
      {#if videoFormatVisible}
        <SelectRow
          icon="film"
          title="Video format"
          description="H.264 plays everywhere; H.265 is smaller"
          options={VIDEO_FORMAT_OPTIONS}
          value={rows.videoFormat}
          onchange={(value) => updateCapture('videoFormat', value)}
        />
      {/if}
      {#if frameRateVisible}
        <SegmentedRow
          icon="fps"
          title="Frame rate"
          description="Frames per second"
          options={RECORDING_FRAME_RATE_OPTIONS}
          value={rows.frameRate}
          onchange={(value) => updateCapture('frameRate', value)}
        />
      {/if}
      {#if qualityVisible}
        <SelectRow
          icon="gauge"
          title="Quality"
          options={QUALITY_OPTIONS}
          value={rows.quality}
          onchange={(value) => updateCapture('quality', value)}
        />
      {/if}
      {#if systemAudioVisible}
        <ToggleRow
          icon="audio"
          title="Record system audio"
          description="Sound playing on this PC"
          checked={rows.systemAudio}
          onchange={(value) => updateCapture('systemAudio', value)}
        />
      {/if}
      {#if microphoneVisible}
        <ToggleRow
          icon="mic"
          title="Record microphone"
          checked={rows.microphone}
          onchange={(value) => updateCapture('microphone', value)}
        />
      {/if}
      {#if microphoneDeviceVisible}
        <SelectRow
          icon="mic"
          title="Microphone device"
          options={microphoneOptions}
          value={rows.microphoneDevice}
          onchange={(value) => updateCapture('microphoneDevice', value)}
        />
      {/if}
      {#if borderVisible}
        <ToggleRow
          icon="border"
          title="Show recording border"
          description="Outline the recorded area (not included in the video)"
          checked={rows.recordingBorder}
          onchange={(value) => updateCapture('recordingBorder', value)}
        />
      {/if}
      {#if ffmpegVisible}
        <FieldRow
          icon="code"
          title="FFmpeg location"
          description="Choose the FFmpeg executable. Snapline finds it automatically when unset."
          mono
          readOnly
          button="Browse…"
          onbutton={() => void pickFfmpegPath()}
          secondaryButton={rows.ffmpegPath ? 'Use bundled' : undefined}
          onsecondarybutton={() => void resetFfmpegPath()}
          value={rows.ffmpegPath}
          placeholder="Automatically detected"
        />
      {/if}
    </SettingsSection>
  {/if}

  {#if showGif}
    <SettingsSection title="GIF">
      {#if gifFrameRateVisible}
        <SegmentedRow
          icon="fps"
          title="Frame rate"
          description="Lower frame rates make much smaller GIFs"
          options={GIF_FRAME_RATE_OPTIONS}
          value={rows.gifFrameRate}
          onchange={(value) => updateCapture('gifFrameRate', value)}
        />
      {/if}
      {#if gifWidthVisible}
        <SelectRow
          icon="width"
          title="Maximum width"
          options={GIF_WIDTH_OPTIONS}
          value={rows.gifWidth}
          onchange={(value) => updateCapture('gifWidth', value)}
        />
      {/if}
      {#if paletteVisible}
        <ToggleRow
          icon="palette"
          title="Optimise colour palette"
          description="Smaller files, slightly slower to encode"
          checked={rows.optimisePalette}
          onchange={(value) => updateCapture('optimisePalette', value)}
        />
      {/if}
    </SettingsSection>
  {/if}
</SettingsPage>

<style>
  .monitor-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 14px;
    padding: 24px 20px 18px;
  }

  .monitor-layout {
    position: relative;
    width: 100%;
  }

  .monitor-position {
    position: absolute;
    padding: 2px;
  }

  .monitor-box {
    display: flex;
    height: 100%;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 2px;
    border-radius: 6px;
    background: var(--subtle);
    border: 1px solid var(--strokeHi);
    color: var(--text3);
    overflow: hidden;
  }

  .monitor-box.on {
    background: var(--accentSoft);
    border: 2px solid var(--accent);
    color: var(--text);
  }

  .monitor-number {
    font-size: 22px;
    font-weight: 600;
  }

  .monitor-resolution {
    font-size: 11px;
    opacity: 0.85;
    text-align: center;
  }

  .monitor-caption {
    font-size: 12px;
    color: var(--text3);
  }
</style>
