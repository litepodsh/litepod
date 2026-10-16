import { describe, expect, it } from 'vitest';

import { canOpenPlatformSettings, registrationSubmitMode } from './cloud-registration';

describe('cloud registration helpers', () => {
	const cloud = { setup_required: false, public_registration_open: true, is_cloud: true };

	it('keeps cloud public registration open after first setup', () => {
		expect(registrationSubmitMode(cloud, false)).toBe('register');
		expect(registrationSubmitMode({ ...cloud, setup_required: true }, false)).toBe('register');
		expect(registrationSubmitMode(cloud, true)).toBe('invite');
	});

	it('limits cloud platform settings to the SysAdmin', () => {
		expect(canOpenPlatformSettings(true, false)).toBe(false);
		expect(canOpenPlatformSettings(true, true)).toBe(true);
		expect(canOpenPlatformSettings(false, false)).toBe(true);
	});
});
