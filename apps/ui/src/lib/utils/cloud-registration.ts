import type { RegistrationStatusResponse } from '#lib/clients/auth-client.js';

export function registrationSubmitMode(
	status: RegistrationStatusResponse,
	hasInvitation: boolean,
): 'setup' | 'register' | 'invite' | 'closed' {
	if (hasInvitation) return 'invite';
	if (!status.public_registration_open) return 'closed';
	return status.is_cloud ? 'register' : 'setup';
}

export function canOpenPlatformSettings(isCloud: boolean, isSysadmin: boolean): boolean {
	return !isCloud || isSysadmin;
}
