import { api } from './base-client';

export type AccountProfile = {
	user_id: string;
	name: string;
	email: string;
	totp_enabled: boolean;
	recovery_codes_remaining: number;
};

export type TotpSetupResponse = {
	secret: string;
	otpauth_url: string;
};

export type TotpEnabledResponse = {
	enabled: boolean;
	recovery_codes: string[];
};

export type SessionDto = {
	id: string;
	created_at: string;
	expires_at: string;
	is_current: boolean;
	user_agent: string | null;
};

export async function getAccount(): Promise<AccountProfile> {
	const resp = await api<AccountProfile>('/api/v1/account');
	return resp.data!;
}

export async function updateAccount(name: string): Promise<AccountProfile> {
	const resp = await api<AccountProfile>('/api/v1/account', {
		method: 'PATCH',
		body: JSON.stringify({ name }),
	});
	return resp.data!;
}

export async function changePassword(
	currentPassword: string,
	newPassword: string,
	confirmPassword: string,
): Promise<void> {
	await api('/api/v1/account/change-password', {
		method: 'POST',
		body: JSON.stringify({
			current_password: currentPassword,
			new_password: newPassword,
			confirm_password: confirmPassword,
		}),
	});
}

export async function deactivateAccount(
	currentPassword: string,
	confirmation: string,
): Promise<void> {
	await api('/api/v1/account', {
		method: 'DELETE',
		body: JSON.stringify({
			current_password: currentPassword,
			confirmation,
		}),
	});
}

export async function setupTotp(): Promise<TotpSetupResponse> {
	const resp = await api<TotpSetupResponse>('/api/v1/account/totp/setup', {
		method: 'POST',
	});
	return resp.data!;
}

export async function verifyTotp(code: string): Promise<TotpEnabledResponse> {
	const resp = await api<TotpEnabledResponse>('/api/v1/account/totp/verify', {
		method: 'POST',
		body: JSON.stringify({ code }),
	});
	return resp.data!;
}

export async function disableTotp(code: string): Promise<void> {
	await api('/api/v1/account/totp/disable', {
		method: 'POST',
		body: JSON.stringify({ code }),
	});
}

export async function regenerateCodes(totpCode: string): Promise<TotpEnabledResponse> {
	const resp = await api<TotpEnabledResponse>('/api/v1/account/recovery-codes', {
		method: 'POST',
		body: JSON.stringify({ code: totpCode }),
	});
	return resp.data!;
}

export async function listSessions(): Promise<SessionDto[]> {
	const resp = await api<SessionDto[]>('/api/v1/account/sessions');
	return resp.data!;
}

export async function revokeSession(sessionId: string): Promise<void> {
	await api(`/api/v1/account/sessions/${sessionId}`, {
		method: 'DELETE',
	});
}

export async function revokeAllSessions(): Promise<void> {
	await api('/api/v1/account/sessions', {
		method: 'DELETE',
	});
}

export async function revokeOtherSessions(): Promise<void> {
	await api('/api/v1/account/sessions/others', {
		method: 'DELETE',
	});
}
