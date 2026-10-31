import { afterEach, describe, expect, it, vi } from 'vitest';

import { domainDnsValidationPath, getMaintenance, updateMaintenance } from './project-client';

describe('domain DNS validation client', () => {
	it('scopes and encodes the validation endpoint', () => {
		expect(domainDnsValidationPath('org one', 'project', 'env', 'app', 'domain id')).toBe(
			'/api/v1/organizations/org%20one/projects/project/environments/env/applications/app/domains/domain%20id/validate-dns',
		);
	});
});

describe('maintenance client', () => {
	afterEach(() => vi.unstubAllGlobals());

	function stubFetch() {
		const fetchMock = vi.fn().mockResolvedValue(
			new Response(
				JSON.stringify({
					error: null,
					data: { enabled: true, custom_html: null, default_html: '<html></html>' },
				}),
				{ status: 200 },
			),
		);
		vi.stubGlobal('fetch', fetchMock);
		return fetchMock;
	}

	const APP =
		'/api/v1/organizations/org/projects/project/environments/env/applications/app/maintenance';

	it('reads the application-scoped maintenance state', async () => {
		const fetchMock = stubFetch();
		const state = await getMaintenance('org', 'project', 'env', 'app');
		expect(state.enabled).toBe(true);
		expect(String(fetchMock.mock.calls[0][0])).toMatch(new RegExp(`${APP}$`));
	});

	it('sends only the keys it was given, so the others are left unchanged', async () => {
		const fetchMock = stubFetch();
		await updateMaintenance('org', 'project', 'env', 'app', { enabled: false });
		const [url, init] = fetchMock.mock.calls[0];
		expect(String(url)).toMatch(new RegExp(`${APP}$`));
		expect(init.method).toBe('PUT');
		expect(JSON.parse(init.body)).toEqual({ enabled: false });
	});
});
