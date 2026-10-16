import { describe, expect, it } from 'vitest';

import { catalogTemplatesPath } from './template-client';

describe('catalogTemplatesPath', () => {
	it('encodes every supported catalog search parameter', () => {
		expect(catalogTemplatesPath({ q: 'mail test', category: 'email', limit: 24 })).toBe(
			'/api/v1/catalog-templates?q=mail+test&category=email&limit=24',
		);
	});

	it('omits empty optional search values', () => {
		expect(catalogTemplatesPath({ q: ' ', category: '' })).toBe('/api/v1/catalog-templates');
	});
});
