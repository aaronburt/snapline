import { describe, expect, it } from 'vitest';
import { resolve, routePaths } from './router';

describe('router', () => {
  it('resolves every route in the plan', () => {
    for (const [path, page] of Object.entries(routePaths)) {
      expect(resolve(`#${path}`).page, path).toBe(page);
    }
  });

  it('keeps a workflow id as a parameter', () => {
    const match = resolve('#/workflows/abc');
    expect(match.page).toBe('workflow-editor');
    expect(match.params.id).toBe('abc');
  });

  it('sends unknown routes to history', () => {
    expect(resolve('#/nonsense').page).toBe('history');
  });
});
