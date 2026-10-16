import { expect, it } from 'vitest';

import { cloudVersionLabel } from './cloud-version-label';

it('labels cloud versions accessibly', () => {
	expect(cloudVersionLabel('v1.2.3', false)).toBe('v1.2.3');
	expect(cloudVersionLabel('v1.2.3', true)).toBe('v1.2.3 — Cloud edition');
});
