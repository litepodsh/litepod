import { describe, expect, it } from 'vitest';
import { osLabel } from './os-label.js';

describe('osLabel', () => {
	it('shows Alpine Linux for the Alpine distribution id', () => {
		expect(osLabel('alpine')).toBe('Alpine Linux');
	});
});
