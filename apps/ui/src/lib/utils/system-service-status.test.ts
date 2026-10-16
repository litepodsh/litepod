import { describe, expect, it } from 'vitest';
import { getSystemServiceStatuses } from './system-service-status';
import { systemVersion } from './system-version-fixture';

describe('getSystemServiceStatuses', () => {
	it('marks litepod API and detected core services available', () => {
		expect(
			getSystemServiceStatuses(systemVersion()).map(({ name, available, description }) => ({
				name,
				available,
				description,
			})),
		).toEqual([
			{ name: 'litepod API', available: true, description: 'Connected' },
			{ name: 'Caddy', available: true, description: 'Detected by Podman' },
			{ name: 'Dragonfly', available: true, description: 'Detected by Podman' },
		]);
	});

	it('marks a missing response and undetected core service unavailable', () => {
		expect(getSystemServiceStatuses(null)[0]).toMatchObject({
			name: 'litepod API',
			available: false,
			description: 'Unavailable',
		});

		const info = systemVersion();
		info.state.core_services.dragonfly.ready = false;
		expect(getSystemServiceStatuses(info)[2]).toMatchObject({
			name: 'Dragonfly',
			available: false,
			description: 'Not detected by Podman',
		});
	});

	it('passes each service uptime through, null when unknown', () => {
		expect(getSystemServiceStatuses(systemVersion()).map((s) => s.uptimeSecs)).toEqual([
			3_600, 86_400, null,
		]);
	});
});
