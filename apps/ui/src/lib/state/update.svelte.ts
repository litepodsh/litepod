import {
	applyUpdate,
	getSystemVersion,
	triggerUpdateCheck,
	type SystemVersion,
} from '#lib/clients/system-client.js';
import { API_URL } from '#lib/clients/base-client.js';
import { isInstallableUpdate, notifySystemUpdateChanged } from '#lib/utils/system-update.js';
import { toast } from '#lib/utils/toast.js';
import { t } from '#lib/i18n/index.svelte.js';

export const GRACE_SECONDS = 20;

// The API only accepts an apply when its own snapshot says update_available.
// The UI shows the button on a wider condition (core-service tags out of date),
// so when the flags disagree we ask the API to re-check and wait for it.
const CHECK_ATTEMPTS = 8;
const CHECK_INTERVAL_MS = 1_500;

type UpdatePhase = 'idle' | 'preparing' | 'grace' | 'watching';

function sleep(ms: number): Promise<void> {
	return new Promise((resolve) => setTimeout(resolve, ms));
}

function createUpdateStore() {
	let applying = $state(false);
	let phase = $state<UpdatePhase>('idle');
	let secondsLeft = $state(0);

	let pollTimer: ReturnType<typeof setInterval> | null = null;
	let deadlineTimer: ReturnType<typeof setTimeout> | null = null;
	let graceTimer: ReturnType<typeof setInterval> | null = null;
	let pollAbort: AbortController | null = null;
	let reloaded = false;

	function clearTimers() {
		if (graceTimer) {
			clearInterval(graceTimer);
			graceTimer = null;
		}
		if (pollTimer) {
			clearInterval(pollTimer);
			pollTimer = null;
		}
		if (deadlineTimer) {
			clearTimeout(deadlineTimer);
			deadlineTimer = null;
		}
		if (pollAbort) {
			pollAbort.abort();
			pollAbort = null;
		}
	}

	async function isHealthy(signal: AbortSignal): Promise<boolean> {
		try {
			const res = await fetch(`${API_URL}/health`, { cache: 'no-store', signal });
			if (!res.ok) return false;
			const body = (await res.json()) as { status?: string };
			return body.status === 'ok';
		} catch {
			return false;
		}
	}

	function reload() {
		if (reloaded) return;
		reloaded = true;
		clearTimers();
		window.location.reload();
	}

	function startWatchdog() {
		clearTimers();
		phase = 'watching';
		secondsLeft = 0;
		pollAbort = new AbortController();
		const { signal } = pollAbort;

		deadlineTimer = setTimeout(reload, 20_000);

		const tick = async () => {
			if (signal.aborted) return;
			if (await isHealthy(signal)) reload();
		};

		void tick();
		pollTimer = setInterval(tick, 1_000);
	}

	// The apply endpoint answers 202 ("scheduled"), so for a few seconds the old
	// container is still up and would answer /health with status ok. Wait out that
	// window before probing, otherwise the watchdog reloads onto the old build.
	function startGrace() {
		clearTimers();
		phase = 'grace';
		secondsLeft = GRACE_SECONDS;
		const endsAt = Date.now() + GRACE_SECONDS * 1_000;

		graceTimer = setInterval(() => {
			const left = Math.ceil((endsAt - Date.now()) / 1_000);
			if (left <= 0) {
				startWatchdog();
				return;
			}
			secondsLeft = left;
		}, 1_000);
	}

	function stopWithMessage(message: string, level: 'error' | 'warning' = 'error') {
		clearTimers();
		applying = false;
		phase = 'idle';
		secondsLeft = 0;
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
		secondsLeft = 0;

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
			if (result.triggered) {
				toast.info(t('common.update.installing'));
				startGrace();
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
	// running: the grace/watchdog timers own the reload.
	function reset() {
		if (applying) return;
		clearTimers();
		phase = 'idle';
		secondsLeft = 0;
		reloaded = false;
	}

	function forceReset() {
		clearTimers();
		applying = false;
		phase = 'idle';
		secondsLeft = 0;
		reloaded = false;
	}

	return {
		get applying() {
			return applying;
		},
		get phase() {
			return phase;
		},
		get secondsLeft() {
			return secondsLeft;
		},
		startUpdate,
		reset,
		forceReset,
	};
}

export const updateState = createUpdateStore();
