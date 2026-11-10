import { tick } from 'svelte';

export async function flush(): Promise<void> {
  for (let round = 0; round < 5; round += 1) {
    await tick();
    await Promise.resolve();
  }
}

export const MIDDAY = new Date(2026, 10, 9, 12, 0, 0);
