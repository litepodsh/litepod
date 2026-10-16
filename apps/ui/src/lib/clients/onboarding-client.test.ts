import { afterEach, describe, expect, it, vi } from 'vitest';
import { applyOnboardingAction, getOnboarding } from './onboarding-client';

const summary = {
	active_guide: 'deploy',
	deploy_route: null,
	panel_position: 'right',
	dismissed: false,
	guides: [],
};

describe('onboarding client', () => {
	afterEach(() => vi.unstubAllGlobals());

	it('uses one route for reads and actions', async () => {
		const fetch = vi.fn().mockResolvedValue({
			ok: true,
			status: 200,
			text: async () => JSON.stringify({ error: null, data: summary }),
		});
		vi.stubGlobal('fetch', fetch);

		await getOnboarding('org/a');
		await applyOnboardingAction('org/a', {
			action: 'set_position',
			position: 'sidebar',
		});

		expect(fetch).toHaveBeenNthCalledWith(
			1,
			expect.stringContaining('/api/v1/organizations/org%2Fa/onboarding'),
			expect.objectContaining({ credentials: 'include' }),
		);
		expect(fetch).toHaveBeenNthCalledWith(
			2,
			expect.stringContaining('/api/v1/organizations/org%2Fa/onboarding'),
			expect.objectContaining({
				method: 'PATCH',
				body: JSON.stringify({ action: 'set_position', position: 'sidebar' }),
			}),
		);
	});
});
