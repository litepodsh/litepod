import type { SystemVersion } from '#lib/clients/system-client.js';

export function systemVersion(overrides: Partial<SystemVersion['state']> = {}): SystemVersion {
	return {
		version: 'v0.1.21',
		git_sha: 'abc123',
		image: 'docker.io/litepod/litepod:v0.1.21',
		image_repo: 'docker.io/litepod/litepod',
		image_tag: 'v0.1.21',
		update_interval_secs: 3600,
		container_name: 'litepod',
		is_cloud: false,
		state: {
			channel: 'stable',
			current_version: 'v0.1.21',
			latest_version: 'v0.1.22',
			update_available: true,
			last_check_at: '2026-07-26T12:00:00Z',
			last_check_succeeded: true,
			last_check_note: '',
			updater_running: false,
			core_services: {
				supported: true,
				caddy: {
					image: 'docker.io/library/caddy',
					current_tag: '2.10.0',
					target_tag: '2.10.0',
					update_available: false,
					ready: true,
					note: '',
					systemd_managed: false,
				},
				dragonfly: {
					image: 'docker.dragonflydb.io/dragonflydb/dragonfly',
					current_tag: 'v1.39.0',
					target_tag: 'v1.39.0',
					update_available: false,
					ready: true,
					note: '',
					systemd_managed: false,
				},
				last_apply_note: '',
			},
			...overrides,
		},
		uptime_secs: { api: 3_600, caddy: 86_400, dragonfly: null },
	};
}
