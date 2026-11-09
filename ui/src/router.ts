import { writable } from 'svelte/store';

export interface RouteSpec {
  pattern: RegExp;
  page: string;
}

export interface RouteMatch {
  path: string;
  page: string;
  params: Record<string, string>;
}

export const routePaths: Record<string, string> = {
  '/panel': 'panel',
  '/history': 'history',
  '/workflows': 'workflows',
  '/workflows/new': 'workflow-editor',
  '/workflows/id': 'workflow-editor',
  '/tools': 'tools',
  '/tools/qr': 'qr',
  '/tools/combine': 'combine',
  '/tools/convert': 'convert',
  '/settings/general': 'settings-general',
  '/settings/capture': 'settings-capture',
  '/settings/hotkeys': 'settings-hotkeys',
  '/settings/uploads': 'settings-uploads',
};

export const routes: RouteSpec[] = [
  { pattern: /^\/panel$/, page: 'panel' },
  { pattern: /^\/history$/, page: 'history' },
  { pattern: /^\/workflows$/, page: 'workflows' },
  { pattern: /^\/workflows\/new$/, page: 'workflow-editor' },
  { pattern: /^\/workflows\/(?<id>[^/]+)$/, page: 'workflow-editor' },
  { pattern: /^\/tools$/, page: 'tools' },
  { pattern: /^\/tools\/qr$/, page: 'qr' },
  { pattern: /^\/tools\/combine$/, page: 'combine' },
  { pattern: /^\/tools\/convert$/, page: 'convert' },
  { pattern: /^\/settings\/general$/, page: 'settings-general' },
  { pattern: /^\/settings\/capture$/, page: 'settings-capture' },
  { pattern: /^\/settings\/hotkeys$/, page: 'settings-hotkeys' },
  { pattern: /^\/settings\/uploads$/, page: 'settings-uploads' },
];

const fallback: RouteMatch = { path: '/history', page: 'history', params: {} };

export function resolve(hash: string): RouteMatch {
  let path = hash.startsWith('#') ? hash.slice(1) : hash;
  if (!path.startsWith('/')) {
    path = `/${path}`;
  }
  for (const route of routes) {
    const match = route.pattern.exec(path);
    if (match) {
      return { path, page: route.page, params: match.groups ?? {} };
    }
  }
  return fallback;
}

export const currentRoute = writable(resolve(window.location.hash));

export function navigate(path: string): void {
  window.location.hash = path.startsWith('/') ? path : `/${path}`;
}

export function startRouter(): void {
  const update = () => currentRoute.set(resolve(window.location.hash));
  window.addEventListener('hashchange', update);
  update();
}
