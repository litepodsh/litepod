import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { ApiError, TWO_FACTOR_REQUIRED, api } from './base-client';

function respondWith(status: number, body: unknown) {
	vi.stubGlobal(
		'fetch',
		vi.fn().mockResolvedValue(new Response(JSON.stringify(body), { status })),
	);
}

function fakeWindow(pathname: string, search = '') {
	const assign = vi.fn();
	vi.stubGlobal('window', { location: { pathname, search, assign } });
	return assign;
}

describe('two-factor requirement', () => {
	beforeEach(() => vi.unstubAllGlobals());
	afterEach(() => vi.unstubAllGlobals());

	it('sends a refused member to 2FA setup and remembers where they were', async () => {
		respondWith(403, { error: 'Two-factor required', data: null, code: TWO_FACTOR_REQUIRED });
		const assign = fakeWindow('/organizations/org-1', '?tab=members');

		await expect(api('/api/v1/organizations/org-1')).rejects.toMatchObject({
			status: 403,
			code: TWO_FACTOR_REQUIRED,
		});
		expect(assign).toHaveBeenCalledWith(
			`/account?tab=security&two_factor=required&next=${encodeURIComponent('/organizations/org-1?tab=members')}`,
		);
	});

	it('does not redirect from the account page, which has to stay reachable', async () => {
		respondWith(403, { error: 'Two-factor required', data: null, code: TWO_FACTOR_REQUIRED });
		const assign = fakeWindow('/account', '?tab=security');

		await expect(api('/api/v1/organizations/org-1')).rejects.toBeInstanceOf(ApiError);
		expect(assign).not.toHaveBeenCalled();
	});

	it('leaves an ordinary 403 alone', async () => {
		respondWith(403, { error: 'Insufficient permissions', data: null });
		const assign = fakeWindow('/organizations/org-1');

		await expect(api('/api/v1/organizations/org-1')).rejects.toMatchObject({
			status: 403,
			code: undefined,
		});
		expect(assign).not.toHaveBeenCalled();
	});
});
