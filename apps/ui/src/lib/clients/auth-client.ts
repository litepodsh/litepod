import { api } from './base-client';
import { invalidateOrganizationsCache } from './organization-client';

export type SetupCheckResponse = {
	setup_required: boolean;
	user_count: number;
};

export type RegistrationStatusResponse = {
	setup_required: boolean;
	public_registration_open: boolean;
	is_cloud: boolean;
	/** Registration shows the Turnstile widget (cloud, release builds). */
	captcha_required: boolean;
};

export type AuthResponse = {
	user: {
		user_id: string;
		email: string;
		name: string;
		is_sysadmin: boolean;
	};
};

export type PendingEmailVerification = {
	email: string;
	accepted_deliveries: number;
	resend_available_at: number;
	delivery_failed: boolean;
	reactivating: boolean;
};

export type MeResponse = {
	user_id: string;
	email: string;
	name: string;
	is_sysadmin: boolean;
	created_at: string;
};

export async function checkSetup(): Promise<SetupCheckResponse> {
	const resp = await api<SetupCheckResponse>('/api/v1/auth/setup');
	return resp.data!;
}

export async function getRegistrationStatus(): Promise<RegistrationStatusResponse> {
	const resp = await api<RegistrationStatusResponse>('/api/v1/auth/registration-status');
	return resp.data!;
}

export async function setup(
	name: string,
	email: string,
	password: string,
	confirmPassword: string,
	setupToken?: string,
): Promise<AuthResponse> {
	const resp = await api<AuthResponse>('/api/v1/auth/setup', {
		method: 'POST',
		body: JSON.stringify({
			name,
			email,
			password,
			confirm_password: confirmPassword,
			setup_token: setupToken,
		}),
	});
	return resp.data!;
}

export async function register(
	username: string,
	email: string,
	password: string,
	confirmPassword: string,
	turnstileToken?: string,
	acceptTerms = false,
): Promise<AuthResponse | PendingEmailVerification> {
	const resp = await api<AuthResponse | PendingEmailVerification>('/api/v1/auth/register', {
		method: 'POST',
		body: JSON.stringify({
			username,
			email,
			password,
			confirm_password: confirmPassword,
			turnstile_token: turnstileToken,
			accept_terms: acceptTerms,
		}),
	});
	return resp.data!;
}

export async function resendEmailVerification(email: string): Promise<void> {
	await api<void>('/api/v1/auth/email-verification/resend', {
		method: 'POST',
		body: JSON.stringify({ email }),
	});
}

export async function confirmEmailVerification(
	email: string,
	code: string,
): Promise<{ verified: boolean; reactivated: boolean }> {
	const resp = await api<{ verified: boolean; reactivated: boolean }>(
		'/api/v1/auth/email-verification/confirm',
		{ method: 'POST', body: JSON.stringify({ email, code }) },
	);
	return resp.data!;
}

/** Returned instead of `AuthResponse` when the account has 2FA on. No session
 *  cookie is set: the challenge has to be exchanged via `completeTotpLogin`. */
export type TotpChallenge = {
	totp_required: true;
	challenge_token: string;
};

export type LoginResult = AuthResponse | TotpChallenge;

export function isTotpChallenge(result: LoginResult): result is TotpChallenge {
	return 'totp_required' in result && result.totp_required;
}

export async function login(
	email: string,
	password: string,
	turnstileToken?: string,
): Promise<LoginResult> {
	const resp = await api<LoginResult>('/api/v1/auth/login', {
		method: 'POST',
		body: JSON.stringify({ email, password, turnstile_token: turnstileToken }),
	});
	return resp.data!;
}

/** Second half of a 2FA login. `code` is an authenticator code or a recovery code. */
export async function completeTotpLogin(
	challengeToken: string,
	code: string,
): Promise<AuthResponse> {
	const resp = await api<AuthResponse>('/api/v1/auth/login/totp', {
		method: 'POST',
		body: JSON.stringify({ challenge_token: challengeToken, code }),
	});
	return resp.data!;
}

/** Resolves to the identity provider's logout URL when the session came from SSO. */
export async function logout(): Promise<string | null> {
	try {
		const resp = await api<{ redirect_url: string | null }>('/api/v1/auth/logout', {
			method: 'POST',
			body: JSON.stringify({}),
		});
		return resp.data?.redirect_url ?? null;
	} finally {
		invalidateOrganizationsCache();
	}
}

export async function revokeAllSessions(): Promise<void> {
	await api<void>('/api/v1/sessions/revoke-all', { method: 'POST' });
}

export async function getMe(): Promise<MeResponse> {
	const resp = await api<MeResponse>('/api/v1/auth/me');
	return resp.data!;
}

export type UserOrganizationMembership = {
	id: string;
	name: string;
	slug: string;
	is_root: boolean;
	role_id: number;
	role_name: string;
	joined_at: string;
};

export type UserWithOrgs = {
	id: string;
	email: string;
	name: string;
	created_at: string;
	/** Locked out by a SysAdmin. */
	disabled: boolean;
	is_sysadmin: boolean;
	organizations: UserOrganizationMembership[];
};

export async function listUsersWithOrgs(): Promise<UserWithOrgs[]> {
	const resp = await api<UserWithOrgs[]>('/api/v1/users');
	return resp.data ?? [];
}

/** Always resolves for a well-formed address — the API answers identically whether
 *  or not an account exists, so the UI must not imply otherwise. */
export async function requestPasswordReset(email: string): Promise<void> {
	await api<void>('/api/v1/auth/password-reset/request', {
		method: 'POST',
		body: JSON.stringify({ email }),
	});
}

export async function confirmPasswordReset(
	token: string,
	password: string,
	confirmPassword: string,
): Promise<void> {
	await api<void>('/api/v1/auth/password-reset/confirm', {
		method: 'POST',
		body: JSON.stringify({ token, password, confirm_password: confirmPassword }),
	});
}
