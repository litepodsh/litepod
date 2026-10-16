import { api } from './base-client';
import type { OrganizationDto } from './organization-client';

export type InvitationOrganization = {
	organization_id: string;
	organization_name: string;
	role_id: number;
	role_name: string;
};

export type Invitation = {
	id: string;
	user_id: string;
	username: string;
	email: string;
	created_by: string;
	created_by_name: string;
	status: 'pending' | 'expired' | 'revoked' | 'accepted';
	expires_at: string;
	created_at: string;
	organizations: InvitationOrganization[];
};

export type CreatedInvitation = { invitation: Invitation; token: string };
export type InvitationDetails = { username: string; email: string; status: 'pending' };
export type CreateInvitationInput = {
	username: string;
	email: string;
	organizations: Array<{ organization_id: string; role_id: number }>;
};

export function organizationInvitationInput(
	organizationId: string,
	username: string,
	email: string,
	roleId: number,
): CreateInvitationInput {
	return {
		username: username.trim(),
		email: email.trim(),
		organizations: [{ organization_id: organizationId, role_id: roleId }],
	};
}

export function invitationsForOrganization(
	invitations: Invitation[],
	organizationId: string,
): Invitation[] {
	return invitations
		.filter((invitation) =>
			invitation.organizations.some(
				(organization) => organization.organization_id === organizationId,
			),
		)
		.map((invitation) => ({
			...invitation,
			organizations: invitation.organizations.filter(
				(organization) => organization.organization_id === organizationId,
			),
		}));
}

export async function inspectInvitation(token: string): Promise<InvitationDetails> {
	const response = await api<InvitationDetails>(`/api/v1/invitations/${encodeURIComponent(token)}`);
	return response.data!;
}

export async function acceptInvitation(
	token: string,
	password: string,
	confirmPassword: string,
	acceptTerms = false,
): Promise<void> {
	await api('/api/v1/auth/invited-register', {
		method: 'POST',
		body: JSON.stringify({
			token,
			password,
			confirm_password: confirmPassword,
			accept_terms: acceptTerms,
		}),
	});
}

export async function listInvitations(): Promise<Invitation[]> {
	const response = await api<Invitation[]>('/api/v1/invitations');
	return response.data ?? [];
}

export async function listInvitationOrganizations(): Promise<OrganizationDto[]> {
	const response = await api<OrganizationDto[]>('/api/v1/invitations/organizations');
	return response.data ?? [];
}

export async function createInvitation(input: CreateInvitationInput): Promise<CreatedInvitation> {
	const response = await api<CreatedInvitation>('/api/v1/invitations', {
		method: 'POST',
		body: JSON.stringify(input),
	});
	return response.data!;
}

export async function revokeInvitation(id: string): Promise<void> {
	await api(`/api/v1/invitations/${encodeURIComponent(id)}/revoke`, { method: 'POST' });
}

export async function regenerateInvitation(id: string): Promise<CreatedInvitation> {
	const response = await api<CreatedInvitation>(
		`/api/v1/invitations/${encodeURIComponent(id)}/regenerate`,
		{ method: 'POST' },
	);
	return response.data!;
}

export function toInvitationUrl(token: string, origin = window.location.origin): string {
	const url = new URL('/register', origin);
	url.searchParams.set('token', token);
	return url.toString();
}
