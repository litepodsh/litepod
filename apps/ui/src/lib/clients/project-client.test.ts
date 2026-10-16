import { describe, expect, it } from 'vitest';

import { domainDnsValidationPath } from './project-client';

describe('domain DNS validation client', () => {
	it('scopes and encodes the validation endpoint', () => {
		expect(domainDnsValidationPath('org one', 'project', 'env', 'app', 'domain id')).toBe(
			'/api/v1/organizations/org%20one/projects/project/environments/env/applications/app/domains/domain%20id/validate-dns',
		);
	});
});
