import { writable } from 'svelte/store';
import { currentRoute } from '../../../router';

export const lastMainRoute = writable('/history');

currentRoute.subscribe((route) => {
  if (!route.page.startsWith('settings-')) {
    lastMainRoute.set(route.path);
  }
});
