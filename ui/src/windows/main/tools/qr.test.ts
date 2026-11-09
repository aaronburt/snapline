import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { expectAccessible } from '../../../test/axe';
import QrTool from './QrTool.svelte';

const ipc = vi.hoisted(() => ({
  ping: vi.fn(),
  qrMake: vi.fn(),
  qrScan: vi.fn(),
  qrCopyText: vi.fn(),
  qrCopyImage: vi.fn(),
  qrSavePng: vi.fn(),
  qrOpen: vi.fn(),
}));

vi.mock('../../../lib/ipc/tool', () => ipc);

const SVG =
  '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 29 29" role="img" aria-label="QR code"><rect width="29" height="29" fill="#ffffff"/><path d="M4 4h1v1h-1z" fill="#000000"/></svg>';

beforeEach(() => {
  ipc.qrMake.mockReset();
  ipc.qrScan.mockReset();
  ipc.qrCopyText.mockReset();
  ipc.qrCopyImage.mockReset();
  ipc.qrSavePng.mockReset();
  ipc.qrOpen.mockReset();
  ipc.qrMake.mockResolvedValue({ svg: SVG, link: null });
  ipc.qrScan.mockResolvedValue({ text: 'hello', link: null, isUrl: false });
  ipc.qrCopyText.mockResolvedValue(undefined);
  ipc.qrCopyImage.mockResolvedValue(undefined);
  ipc.qrSavePng.mockResolvedValue('C:\\Captures\\qr.png');
  ipc.qrOpen.mockResolvedValue(undefined);
});

afterEach(cleanup);

async function openMake(): Promise<HTMLElement> {
  await fireEvent.click(screen.getByRole('radio', { name: 'Make' }));
  return screen.getByLabelText('Text or link');
}

describe('QrTool', () => {
  it('renders the Scan and Make tabs with Scan selected', async () => {
    const { container } = render(QrTool);
    await expectAccessible(container);
    expect(screen.getByText('QR code')).not.toBeNull();
    expect(screen.getByRole('radio', { name: 'Scan' }).getAttribute('aria-checked')).toBe('true');
    expect(screen.getByRole('radio', { name: 'Make' }).getAttribute('aria-checked')).toBe('false');
    expect(screen.getByRole('button', { name: 'Select area' })).not.toBeNull();
  });

  it('switches to Make and disables the output buttons until there is a preview', async () => {
    const { container } = render(QrTool);
    await openMake();
    expect(screen.getByText('Text or link')).not.toBeNull();
    expect(container.querySelector('.preview [role="img"]')).toBeNull();
    expect(screen.getByRole('button', { name: 'Copy image' }).hasAttribute('disabled')).toBe(true);
    expect(screen.getByRole('button', { name: 'Save as PNG' }).hasAttribute('disabled')).toBe(true);
  });

  it('asks the service for an SVG as the text changes', async () => {
    const { container } = render(QrTool);
    const input = await openMake();
    await fireEvent.input(input, { target: { value: 'https://example.com' } });
    await waitFor(() => expect(ipc.qrMake).toHaveBeenCalledWith('https://example.com'));
    await waitFor(() => expect(container.querySelector('.preview svg')).not.toBeNull());
    expect(container.querySelector('.preview svg')?.getAttribute('aria-label')).toBe('QR code');
  });

  it('copies the made image through the tool service', async () => {
    render(QrTool);
    const input = await openMake();
    await fireEvent.input(input, { target: { value: 'hello' } });
    await waitFor(() => expect(ipc.qrMake).toHaveBeenCalled());
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Copy image' }).hasAttribute('disabled')).toBe(
        false,
      ),
    );
    await fireEvent.click(screen.getByRole('button', { name: 'Copy image' }));
    await waitFor(() => expect(ipc.qrCopyImage).toHaveBeenCalledWith('hello'));
    expect(await screen.findByText('Image copied')).not.toBeNull();
  });

  it('saves a PNG and reports the path', async () => {
    render(QrTool);
    const input = await openMake();
    await fireEvent.input(input, { target: { value: 'hello' } });
    await waitFor(() => expect(ipc.qrMake).toHaveBeenCalled());
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Save as PNG' }).hasAttribute('disabled')).toBe(
        false,
      ),
    );
    await fireEvent.click(screen.getByRole('button', { name: 'Save as PNG' }));
    await waitFor(() => expect(ipc.qrSavePng).toHaveBeenCalledWith('hello'));
    expect(await screen.findByText('Saved to C:\\Captures\\qr.png')).not.toBeNull();
  });

  it('scans an area and offers Copy and Open for a link', async () => {
    ipc.qrScan.mockResolvedValue({
      text: 'https://example.com',
      link: 'https://example.com',
      isUrl: true,
    });
    render(QrTool);
    await fireEvent.click(screen.getByRole('button', { name: 'Select area' }));
    await waitFor(() => expect(ipc.qrScan).toHaveBeenCalledTimes(1));
    expect(await screen.findByLabelText('QR code text')).not.toBeNull();
    expect(screen.getByText('https://example.com')).not.toBeNull();
    await fireEvent.click(screen.getByRole('button', { name: 'Copy' }));
    await waitFor(() => expect(ipc.qrCopyText).toHaveBeenCalledWith('https://example.com'));
    expect(await screen.findByText('Text copied')).not.toBeNull();
    await fireEvent.click(screen.getByRole('button', { name: 'Open' }));
    await waitFor(() => expect(ipc.qrOpen).toHaveBeenCalledWith('https://example.com'));
  });

  it('does not offer Open for text that is not a link', async () => {
    render(QrTool);
    await fireEvent.click(screen.getByRole('button', { name: 'Select area' }));
    expect(await screen.findByText('hello')).not.toBeNull();
    expect(screen.queryByRole('button', { name: 'Open' })).toBeNull();
  });

  it('shows the service message when the selection overlay is not ready', async () => {
    ipc.qrScan.mockRejectedValue({
      code: 'internal',
      message: 'This part of Snapline is not built yet.',
    });
    render(QrTool);
    await fireEvent.click(screen.getByRole('button', { name: 'Select area' }));
    expect(await screen.findByText('This part of Snapline is not built yet.')).not.toBeNull();
    expect(screen.queryByLabelText('QR code text')).toBeNull();
  });

  it('shows validation messages from the QR service', async () => {
    ipc.qrMake.mockRejectedValue({
      code: 'invalid',
      message: 'The text is too long for a QR code. Shorten it and try again.',
    });
    render(QrTool);
    const input = await openMake();
    await fireEvent.input(input, { target: { value: 'too long' } });
    expect(
      await screen.findByText('The text is too long for a QR code. Shorten it and try again.'),
    ).not.toBeNull();
  });
});
