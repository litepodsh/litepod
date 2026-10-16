import type { SystemVersion } from '#lib/clients/system-client.js';

export const SYSTEM_UPDATE_CHANGED_EVENT = 'litepod:system-update-changed';

export function isInstallableUpdate(info: SystemVersion | null): boolean {
	if (!info?.state.last_check_succeeded || !info.state.core_services.supported) return false;

	const { state } = info;
	const { caddy, dragonfly } = state.core_services;
	if (!state.latest_version || !caddy.target_tag || !dragonfly.target_tag) return false;

	return (
		state.update_available ||
		info.version !== state.latest_version ||
		caddy.update_available ||
		caddy.current_tag !== caddy.target_tag ||
		dragonfly.update_available ||
		dragonfly.current_tag !== dragonfly.target_tag
	);
}

export function notifySystemUpdateChanged(): void {
	if (typeof window !== 'undefined') {
		window.dispatchEvent(new Event(SYSTEM_UPDATE_CHANGED_EVENT));
	}
}
