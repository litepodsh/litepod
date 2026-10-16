import { describe, expect, it } from 'vitest';

import { can, type PermissionSet } from './permission-client';

const set: PermissionSet = {
	org: ['org.read'],
	projects: { p1: ['app.deploy'] },
	environments: { e2: ['app.read'] },
};

describe('can', () => {
	it('grants org permissions everywhere', () => {
		expect(can(set, 'org.read')).toBe(true);
		expect(can(set, 'org.read', { projectId: 'p9' })).toBe(true);
	});

	it('inherits project grants into its environments', () => {
		expect(can(set, 'app.deploy', { projectId: 'p1' })).toBe(true);
		expect(can(set, 'app.deploy', { projectId: 'p1', environmentId: 'e1' })).toBe(true);
		expect(can(set, 'app.deploy', { projectId: 'p2' })).toBe(false);
	});

	it('does not lift environment grants to the project', () => {
		expect(can(set, 'app.read', { projectId: 'p2', environmentId: 'e2' })).toBe(true);
		expect(can(set, 'app.read', { projectId: 'p2' })).toBe(false);
	});

	it('denies everything without a set', () => {
		expect(can(null, 'org.read')).toBe(false);
	});
});
