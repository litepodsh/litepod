import { expect, it } from 'vitest';
import { organizationPathFor } from './selected-organization';

it('moves an organization page to the chosen organization', () => {
	expect(organizationPathFor('/organizations/mine/servers', 'theirs')).toBe(
		'/organizations/theirs/servers',
	);
	expect(organizationPathFor('/organizations/mine', 'theirs')).toBe('/organizations/theirs');
});

it('drops ids that belong to the previous organization', () => {
	expect(organizationPathFor('/organizations/mine/servers/s1', 'theirs')).toBe(
		'/organizations/theirs/servers',
	);
});

it('leaves pages outside an organization alone', () => {
	expect(organizationPathFor('/home', 'theirs')).toBe('/home');
});
