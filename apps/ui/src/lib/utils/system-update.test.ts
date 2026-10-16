import { describe, expect, it } from 'vitest';
import { isInstallableUpdate } from './system-update';
import { systemVersion } from './system-version-fixture';

describe('isInstallableUpdate', () => {
	it('accepts a verified litepod update with complete targets', () => {
		expect(isInstallableUpdate(systemVersion())).toBe(true);
	});

	it.each([
		['Caddy', 'caddy'],
		['Dragonfly', 'dragonfly'],
	] as const)('accepts a verified %s-only update', (_label, service) => {
		const info = systemVersion({
			latest_version: 'v0.1.21',
			update_available: false,
		});
		info.state.core_services[service].target_tag += '-new';
		info.state.core_services[service].update_available = true;

		expect(isInstallableUpdate(info)).toBe(true);
	});

	it('rejects a failed check', () => {
		expect(isInstallableUpdate(systemVersion({ last_check_succeeded: false }))).toBe(false);
	});

	it('rejects unsupported core-service updates', () => {
		const info = systemVersion();
		info.state.core_services.supported = false;
		expect(isInstallableUpdate(info)).toBe(false);
	});

	it.each(['latest_version', 'caddy', 'dragonfly'] as const)(
		'rejects a missing %s target',
		(target) => {
			const info = systemVersion();
			if (target === 'latest_version') info.state.latest_version = '';
			else info.state.core_services[target].target_tag = '';

			expect(isInstallableUpdate(info)).toBe(false);
		},
	);

	it('rejects a successful check when every target is current', () => {
		expect(
			isInstallableUpdate(
				systemVersion({
					latest_version: 'v0.1.21',
					update_available: false,
				}),
			),
		).toBe(false);
	});

	it('rejects absent system information', () => {
		expect(isInstallableUpdate(null)).toBe(false);
	});
});
