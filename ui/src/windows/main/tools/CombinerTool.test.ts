import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { expectAccessible } from '../../../test/axe';
import type { HistoryCaptureDto } from '../../../lib/ipc/bindings/HistoryCaptureDto';
import CombinerTool from './CombinerTool.svelte';

const mocks = vi.hoisted(() => ({
  call: vi.fn(),
  page: vi.fn(),
}));

vi.mock('../../../lib/ipc/core', () => ({ call: mocks.call }));
vi.mock('../../../lib/ipc/history', () => ({ historyPage: mocks.page }));

function capture(id: number): HistoryCaptureDto {
  return {
    id,
    kind: 'image',
    name: `capture-${id}.png`,
    bytes: 412 * 1024,
    width: 1280,
    height: 720,
    durationMs: null,
    sourceApp: 'chrome.exe',
    createdAt: Date.now(),
    folder: 'Pictures\\Snapline\\2026-11',
    upload: null,
  };
}

interface RequestShape {
  kind: string;
  data?: {
    sources?: Array<{ kind: string; data: { id: number } }>;
    options?: {
      layout: string;
      spacing: number;
      align: string;
      background: { kind: string; value?: string };
    };
  };
}

beforeEach(() => {
  mocks.call.mockReset();
  mocks.page.mockReset();
  mocks.page.mockResolvedValue({ items: [capture(1), capture(2)], next: null });
  mocks.call.mockImplementation(async (_command: string, request: RequestShape) => {
    if (request.kind === 'combinePreview') {
      return { kind: 'combinePreviewed', data: { job: 7 } };
    }
    if (request.kind === 'combinePickFiles') {
      return { kind: 'combinePickedFiles', data: { files: [{ id: 3, name: 'picked.png' }] } };
    }
    if (request.kind === 'combineSave') {
      return { kind: 'combineSaved', data: { captureId: 12 } };
    }
    throw { code: 'internal', message: 'Unexpected request.' };
  });
});

afterEach(cleanup);

function previewRequests(): RequestShape[] {
  return mocks.call.mock.calls
    .map((call) => call[1] as RequestShape)
    .filter((request) => request.kind === 'combinePreview');
}

async function pick(name: string): Promise<void> {
  await fireEvent.click(screen.getByRole('button', { name: new RegExp(name) }));
}

describe('CombinerTool', () => {
  it('renders the picker and the default controls', async () => {
    const { container } = render(CombinerTool);
    await screen.findByText('capture-1.png');
    await expectAccessible(container);
    expect(screen.getByText('Image combiner')).not.toBeNull();
    expect(await screen.findByText('capture-1.png')).not.toBeNull();
    expect(screen.getByText('capture-2.png')).not.toBeNull();
    expect(screen.getByRole('radio', { name: 'Side by side' }).getAttribute('aria-checked')).toBe('true');
    expect(screen.getByRole('radio', { name: 'Start' }).getAttribute('aria-checked')).toBe('true');
    expect(screen.getByRole('radio', { name: 'Transparent' }).getAttribute('aria-checked')).toBe(
      'true',
    );
    expect(screen.getByText('8 px')).not.toBeNull();
  });

  it('asks for a preview when a History capture is picked and shows the combine scheme', async () => {
    render(CombinerTool);
    await screen.findByText('capture-1.png');
    await pick('capture-1.png');
    await waitFor(() => expect(previewRequests().length).toBeGreaterThan(0));
    const request = previewRequests().at(-1);
    expect(request?.data?.sources).toEqual([{ kind: 'history', data: { id: 1 } }]);
    expect(request?.data?.options?.layout).toBe('sideBySide');
    const image = await screen.findByAltText('Combined preview');
    expect(image.getAttribute('src')).toBe('http://combine.localhost/7');
  });

  it('sends the chosen background and spacing', async () => {
    render(CombinerTool);
    await screen.findByText('capture-1.png');
    await pick('capture-1.png');
    await waitFor(() => expect(previewRequests().length).toBeGreaterThan(0));
    await fireEvent.click(screen.getByRole('radio', { name: 'White' }));
    await waitFor(() =>
      expect(previewRequests().at(-1)?.data?.options?.background).toEqual({ kind: 'white' }),
    );
    const slider = screen.getByLabelText('Spacing');
    await fireEvent.input(slider, { target: { value: '24' } });
    await waitFor(() => expect(previewRequests().at(-1)?.data?.options?.spacing).toBe(24));
  });

  it('shows a hex field for a custom background', async () => {
    render(CombinerTool);
    expect(screen.queryByText('Custom colour')).toBeNull();
    await fireEvent.click(screen.getByRole('radio', { name: 'Custom' }));
    expect(await screen.findByText('Custom colour')).not.toBeNull();
    await screen.findByText('capture-1.png');
    await pick('capture-1.png');
    await waitFor(() =>
      expect(previewRequests().at(-1)?.data?.options?.background).toEqual({
        kind: 'custom',
        value: '#ffffff',
      }),
    );
    const input = screen.getByLabelText('Custom colour');
    await fireEvent.input(input, { target: { value: '#102030' } });
    await waitFor(() =>
      expect(previewRequests().at(-1)?.data?.options?.background).toEqual({
        kind: 'custom',
        value: '#102030',
      }),
    );
  });

  it('adds picked files through the native dialog request', async () => {
    render(CombinerTool);
    await screen.findByText('capture-1.png');
    await fireEvent.click(screen.getByRole('button', { name: 'Add files…' }));
    await waitFor(() =>
      expect(mocks.call).toHaveBeenCalledWith('ipc_tool', { kind: 'combinePickFiles' }),
    );
    expect(await screen.findByText('picked.png')).not.toBeNull();
    await waitFor(() =>
      expect(previewRequests().at(-1)?.data?.sources).toEqual([{ kind: 'picked', data: { id: 3 } }]),
    );
  });

  it('reorders with the move controls', async () => {
    render(CombinerTool);
    await screen.findByText('capture-1.png');
    await pick('capture-1.png');
    await pick('capture-2.png');
    await waitFor(() => expect(previewRequests().at(-1)?.data?.sources?.length).toBe(2));
    await fireEvent.click(screen.getByRole('button', { name: 'Move capture-2.png up' }));
    await waitFor(() =>
      expect(previewRequests().at(-1)?.data?.sources).toEqual([
        { kind: 'history', data: { id: 2 } },
        { kind: 'history', data: { id: 1 } },
      ]),
    );
  });

  it('saves and reports that the result is in History', async () => {
    render(CombinerTool);
    await screen.findByText('capture-1.png');
    await pick('capture-1.png');
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Save' }).hasAttribute('disabled')).toBe(false),
    );
    await fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(mocks.call).toHaveBeenCalledWith('ipc_tool', {
      kind: 'combineSave',
      data: {
        sources: [{ kind: 'history', data: { id: 1 } }],
        options: {
          layout: 'sideBySide',
          spacing: 8,
          align: 'start',
          background: { kind: 'transparent' },
        },
      },
    }));
    expect(await screen.findByText('Saved to History')).not.toBeNull();
  });

  it('shows preview errors from the service', async () => {
    mocks.call.mockImplementation(async (_command: string, request: RequestShape) => {
      if (request.kind === 'combinePreview') {
        throw { code: 'invalid', message: 'The combined image is too large.' };
      }
      return { kind: 'combinePickedFiles', data: { files: [] } };
    });
    render(CombinerTool);
    await screen.findByText('capture-1.png');
    await pick('capture-1.png');
    expect(await screen.findByText('The combined image is too large.')).not.toBeNull();
  });

  it('shows the empty History message', async () => {
    mocks.page.mockResolvedValue({ items: [], next: null });
    render(CombinerTool);
    expect(await screen.findByText('No image captures yet.')).not.toBeNull();
  });
});
