import axe from 'axe-core';
import { expect } from 'vitest';

const fragmentRules = {
  region: { enabled: false },
  bypass: { enabled: false },
  'page-has-heading-one': { enabled: false },
  'html-has-lang': { enabled: false },
  'landmark-one-main': { enabled: false },
};

export async function expectAccessible(container: HTMLElement): Promise<void> {
  const results = await axe.run(container, { rules: fragmentRules });
  expect(
    results.violations.map(
      (violation) =>
        `${violation.id}: ${violation.help} [${violation.nodes
          .map((node) => node.target.join(' '))
          .join(', ')}]`,
    ),
  ).toEqual([]);
}
