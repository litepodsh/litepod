import {
	applyUpdate,
	getSystemVersion,
	triggerUpdateCheck,
	type SystemVersion,
} from '#lib/clients/system-client.js';
import { isInstallableUpdate, notifySystemUpdateChanged } from '#lib/utils/system-update.js';
import { toast } from '#lib/utils/toast.js';
import { t } from '#lib/i18n/index.svelte.js';

// The API only accepts an apply when its own snapshot says update_available.
// The UI shows the button on a wider condition (core-service tags out of date),
// so when the flags disagree we ask the API to re-check and wait for it.
const CHECK_ATTEMPTS = 8;
const CHECK_INTERVAL_MS = 1_500;

// Poll interval and deadline for waiting on the replacement container. The apply
// endpoint answers 202 while the old container keeps serving during the backup,
// image pull and self-recreation, so a health probe would reload onto the old
// build. We poll the version instead and reload only once it has flipped.
const WATCH_POLL_MS = 2_000;
const WATCH_LIMIT_MS = 15 * 60_000;
const BACKUP_LIMIT_MS = 40 * 60_000;

type UpdatePhase = 'idle' | 'preparing' | 'watching';

function sleep(ms: number): Promise<void> {
	return new Promise((resolve) => setTimeout(resolve, ms));
}

function createUpdateStore() {
	let applying = $state(false);
	let phase = $state<UpdatePhase>('idle');

	let pollTimer: ReturnType<typeof setInterval> | null = null;
	let reloaded = false;

	function clearTimers() {
		if (pollTimer) {
			clearInterval(pollTimer);
			pollTimer = null;
		}
	}

	function reload() {
		if (reloaded) return;
		reloaded = true;
		clearTimers();
		window.location.reload();
	}

	// Wait for the replacement container to come online by watching the build
	// identity, not a health probe. The old container keeps answering /health the
	// whole time the backup runs and the new image pulls, so a health-based reload
	// lands on the old build. `version` flips the instant the replacement serves.
	//
	// A backup that failed before the install cancels the update; the background
	// apply writes that into `last_check_note`, which we surface and stop on.
	function startWatching(startedVersion: string, withBackup: boolean) {
		clearTimers();
		phase = 'watching';
		const endsAt = Date.now() + (withBackup ? BACKUP_LIMIT_MS : WATCH_LIMIT_MS);
		let inFlight = false;
		pollTimer = setInterval(async () => {
			if (inFlight) return;
			inFlight = true;
			try {
				const info = await getSystemVersion();
				const note = info.state.last_check_note ?? '';
				if (note.startsWith('update cancelled')) {
					stopWithMessage(note);
					return;
				}
				if (info.version && info.version !== startedVersion) {
					reload();
					return;
				}
				if (Date.now() > endsAt) {
					stopWithMessage(t('common.update.install_failed'));
				}
			} catch {
				// The API stops answering while the replacement boots. Keep polling:
				// the new build reports a different version the moment it serves.
				if (Date.now() > endsAt) {
					stopWithMessage(t('common.update.install_failed'));
				}
			} finally {
				inFlight = false;
			}
		}, WATCH_POLL_MS);
	}

	function stopWithMessage(message: string, level: 'error' | 'warning' = 'error') {
		clearTimers();
		applying = false;
		phase = 'idle';
		if (level === 'warning') toast.warning(message);
		else toast.error(message);
	}

	// Always installs from a freshly-read API snapshot instead of whatever the
	// caller had rendered, so every entry point (sidebar, system page) sends the
	// exact targets the API is currently willing to accept.
	async function resolveTarget(): Promise<SystemVersion | null> {
		let info = await getSystemVersion();
		if (info.state.update_available) return info;
		if (!isInstallableUpdate(info)) return null;

		await triggerUpdateCheck();
		for (let attempt = 0; attempt < CHECK_ATTEMPTS; attempt++) {
			await sleep(CHECK_INTERVAL_MS);
			info = await getSystemVersion();
			if (info.state.update_available) return info;
		}
		return null;
	}

	async function startUpdate() {
		if (applying) return;
		applying = true;
		phase = 'preparing';

		let info: SystemVersion | null;
		try {
			info = await resolveTarget();
		} catch (error) {
			stopWithMessage(
				error instanceof Error ? error.message : t('common.update.state_read_failed'),
			);
			return;
		}

		if (!info) {
			notifySystemUpdateChanged();
			stopWithMessage(t('common.update.none_pending'), 'warning');
			return;
		}

		const state = info.state;
		if (!state.latest_version || !state.channel) {
			stopWithMessage(t('common.update.target_incomplete'));
			return;
		}

		const core = state.core_services;
		if (!core.caddy.target_tag || !core.dragonfly.target_tag) {
			stopWithMessage(t('common.update.core_targets_incomplete'));
			return;
		}

		try {
			const result = await applyUpdate(state.latest_version, state.channel, {
				caddy: core.caddy.target_tag,
				dragonfly: core.dragonfly.target_tag,
			});
			if (result.triggered && result.backup_started) {
				toast.info(t('common.update.backing_up'));
				startWatching(info.version, true);
			} else if (result.triggered) {
				toast.info(t('common.update.installing'));
				startWatching(info.version, false);
			} else {
				notifySystemUpdateChanged();
				stopWithMessage(result.note, 'warning');
			}
		} catch (error) {
			notifySystemUpdateChanged();
			stopWithMessage(error instanceof Error ? error.message : t('common.update.install_failed'));
		}
	}

	// An unmounting entry point must not tear down an install that is already
	// running: the watcher owns the reload.
	function reset() {
		if (applying) return;
		clearTimers();
		phase = 'idle';
		reloaded = false;
	}

	function forceReset() {
		clearTimers();
		applying = false;
		phase = 'idle';
		reloaded = false;
	}

	return {
		get applying() {
			return applying;
		},
		get phase() {
			return phase;
		},
		startUpdate,
		reset,
		forceReset,
	};
}

export const updateState = createUpdateStore();
