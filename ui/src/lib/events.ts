import { listen } from '@tauri-apps/api/event';
import type { UnlistenFn } from '@tauri-apps/api/event';
import type { AppEvent } from './ipc/bindings/AppEvent';

export function onEvent(handler: (event: AppEvent) => void): Promise<UnlistenFn> {
  return listen<AppEvent>('snapline://event', (event) => handler(event.payload));
}
