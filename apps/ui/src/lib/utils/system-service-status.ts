import type { CoreServiceUpdateState, SystemVersion } from '#lib/clients/system-client.js';

export type SystemServiceStatus = {
	name: 'litepod API' | 'Caddy' | 'Dragonfly';
	currentTag: string;
	targetTag: string;
	available: boolean;
	description: 'Connected' | 'Unavailable' | 'Detected by Podman' | 'Not detected by Podman';
	uptimeSecs: number | null;
};

function coreServiceStatus(
	name: 'Caddy' | 'Dragonfly',
	service: CoreServiceUpdateState,
	uptimeSecs: number | null,
): SystemServiceStatus {
	return {
		name,
		currentTag: service.current_tag,
		targetTag: service.target_tag,
		available: service.ready,
		description: service.ready ? 'Detected by Podman' : 'Not detected by Podman',
		uptimeSecs,
	};
}

export function getSystemServiceStatuses(info: SystemVersion | null): SystemServiceStatus[] {
	const api: SystemServiceStatus = {
		name: 'litepod API',
		currentTag: info?.version ?? '',
		targetTag: info?.state.latest_version ?? '',
		available: info !== null,
		description: info ? 'Connected' : 'Unavailable',
		uptimeSecs: info?.uptime_secs?.api ?? null,
	};

	if (!info) return [api];

	const { caddy, dragonfly } = info.state.core_services;
	const uptime = info.uptime_secs;
	return [
		api,
		coreServiceStatus('Caddy', caddy, uptime?.caddy ?? null),
		coreServiceStatus('Dragonfly', dragonfly, uptime?.dragonfly ?? null),
	];
}
