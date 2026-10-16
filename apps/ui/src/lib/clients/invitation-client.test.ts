import { describe, expect, it } from 'vitest';

import {
	invitationsForOrganization,
	organizationInvitationInput,
	toInvitationUrl,
	type Invitation,
} from './invitation-client';

function invitationFixture(overrides: Partial<Invitation> = {}): Invitation {
	return {
		id: 'invite-1',
		user_id: 'user-1',
		username: 'Mara',
		email: 'mara@example.com',
		created_by: 'admin-1',
		created_by_name: 'Admin',
		status: 'pending',
		expires_at: '2026-09-09 12:00:00',
		created_at: '2026-09-02 12:00:00',
		organizations: [],
		...overrides,
	};
}

describe('invitation client', () => {
	it('builds a registration link from the supplied origin', () => {
		expect(toInvitationUrl('abc', 'https://litepod.test')).toBe(
			'https://litepod.test/register?token=abc',
		);
	});

	it('builds exactly one assignment from route organization context', () => {
		expect(organizationInvitationInput('org-7', ' Mara ', ' mara@example.com ', 2)).toEqual({
			username: 'Mara',
			email: 'mara@example.com',
			organizations: [{ organization_id: 'org-7', role_id: 2 }],
		});
	});

	it('projects invitation rows to the open organization only', () => {
		const invitation = invitationFixture({
			organizations: [
				{
					organization_id: 'org-7',
					organization_name: 'North',
					role_id: 2,
					role_name: 'Editor',
				},
				{
					organization_id: 'org-8',
					organization_name: 'South',
					role_id: 3,
					role_name: 'Viewer',
				},
			],
		});

		expect(invitationsForOrganization([invitation], 'org-7')[0].organizations).toEqual([
			invitation.organizations[0],
		]);
		expect(invitationsForOrganization([invitation], 'missing')).toEqual([]);
	});
});
