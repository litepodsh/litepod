import { describe, expect, it } from 'vitest';
import { displayApplicationIconUrl } from './application-card-icon';

describe('displayApplicationIconUrl', () => {
	it('uses a catalog URL until that application icon fails', () => {
		expect(
			displayApplicationIconUrl(
				'app-1',
				null,
				'https://templates.litepod.sh/api/v1/templates/mailpit/icon',
				new Set(),
			),
		).toBe('https://templates.litepod.sh/api/v1/templates/mailpit/icon');
		expect(
			displayApplicationIconUrl(
				'app-1',
				null,
				'https://templates.litepod.sh/api/v1/templates/mailpit/icon',
				new Set(['app-1']),
			),
		).toBeNull();
	});
	it('falls back without a URL', () => {
		expect(displayApplicationIconUrl('app-2', null, null, new Set())).toBeNull();
	});
	it('rejects a non-http(s) URL like file:// or an empty string', () => {
		expect(
			displayApplicationIconUrl('app-4', 'file:///home/user/rustrak.png', null, new Set()),
		).toBeNull();
		expect(displayApplicationIconUrl('app-4', '   ', null, new Set())).toBeNull();
	});
	it('prefers the user override over the catalog default', () => {
		expect(
			displayApplicationIconUrl(
				'app-3',
				'/api/v1/organizations/o/projects/p/environments/e/applications/app-3/icon/x.png',
				'https://templates.litepod.sh/api/v1/templates/mailpit/icon',
				new Set(),
			),
		).toBe('/api/v1/organizations/o/projects/p/environments/e/applications/app-3/icon/x.png');
	});
});
