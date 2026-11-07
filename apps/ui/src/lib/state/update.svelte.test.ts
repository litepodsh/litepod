import { afterEach, describe, expect, it, vi } from 'vitest';
import { systemVersion } from '#lib/utils/system-version-fixture.js';

const { applyUpdate, getSystemVersion, triggerUpdateCheck, toast } = vi.hoisted(() => ({
	applyUpdate: vi.fn(),
	getSystemVersion: vi.fn(),
	triggerUpdateCheck: vi.fn(),
	toast: {
		error: vi.fn(),
		info: vi.fn(),
		warning: vi.fn(),
	},
}));

vi.mock('#lib/clients/system-client.js', () => ({
	applyUpdate,
	getSystemVersion,
	triggerUpdateCheck,
}));
vi.mock('#lib/clients/base-client', () => ({ API_URL: 'http://api.test' }));
vi.mock('#lib/utils/toast.js', () => ({ toast }));

const { updateState } = await import('./update.svelte');

afterEach(() => {
	updateState.forceReset();
	vi.clearAllMocks();
});

describe('updateState.startUpdate', () => {
	it('applies the targets read fresh from the API and starts the wait state', async () => {
		getSystemVersion.mockResolvedValue(systemVersion());
		applyUpdate.mockResolvedValue({ triggered: true, note: 'scheduled' });

		await updateState.startUpdate();

		expect(getSystemVersion).toHaveBeenCalled();
		expect(applyUpdate).toHaveBeenCalledWith('v0.1.22', 'stable', {
			caddy: '2.10.0',
			dragonfly: 'v1.39.0',
		});
		expect(toast.info).toHaveBeenCalledWith(
			'Installing update. Only the control plane restarts (~10s); your deployed apps keep running.',
		);
		expect(updateState.applying).toBe(true);
		expect(updateState.phase).toBe('watching');
	});

	it('ignores a stale caller snapshot and installs what the API reports now', async () => {
		const fresh = systemVersion({ latest_version: 'v0.1.40' });
		getSystemVersion.mockResolvedValue(fresh);
		applyUpdate.mockResolvedValue({ triggered: true, note: 'scheduled' });

		await updateState.startUpdate();

		expect(applyUpdate).toHaveBeenCalledWith('v0.1.40', 'stable', expect.anything());
	});

	it('rejects incomplete core-service targets before applying', async () => {
		const info = systemVersion();
		info.state.core_services.caddy.target_tag = '';
		getSystemVersion.mockResolvedValue(info);

		await updateState.startUpdate();

		expect(applyUpdate).not.toHaveBeenCalled();
		expect(toast.error).toHaveBeenCalledWith(
			'The shown core-service targets are incomplete. Check again before installing.',
		);
		expect(updateState.applying).toBe(false);
	});

	it('triggers a server check when only the core-service tags are behind', async () => {
		const stale = systemVersion({ update_available: false });
		stale.state.core_services.caddy.target_tag = '2.11.0';
		const checked = systemVersion({ update_available: true });
		checked.state.core_services.caddy.current_tag = '2.10.0';
		checked.state.core_services.caddy.target_tag = '2.11.0';

		getSystemVersion.mockResolvedValueOnce(stale).mockResolvedValue(checked);
		triggerUpdateCheck.mockResolvedValue({ triggered: true, note: 'scheduled' });
		applyUpdate.mockResolvedValue({ triggered: true, note: 'scheduled' });

		await updateState.startUpdate();

		expect(triggerUpdateCheck).toHaveBeenCalled();
		expect(applyUpdate).toHaveBeenCalledWith('v0.1.22', 'stable', {
			caddy: '2.11.0',
			dragonfly: 'v1.39.0',
		});
		expect(updateState.phase).toBe('watching');
	});

	it('warns instead of applying when the API reports nothing pending', async () => {
		getSystemVersion.mockResolvedValue(
			systemVersion({ update_available: false, latest_version: 'v0.1.21' }),
		);

		await updateState.startUpdate();

		expect(applyUpdate).not.toHaveBeenCalled();
		expect(triggerUpdateCheck).not.toHaveBeenCalled();
		expect(toast.warning).toHaveBeenCalledWith(
			'No update is pending right now. Run a check and try again.',
		);
		expect(updateState.applying).toBe(false);
	});

	it('waits for the backup by watching the version, then reloads once it changes', async () => {
		vi.useFakeTimers();
		try {
			getSystemVersion.mockResolvedValueOnce(systemVersion());
			applyUpdate.mockResolvedValue({ triggered: true, backup_started: true, note: 'backup' });

			await updateState.startUpdate();

			expect(toast.info).toHaveBeenCalledWith(
				'Backing up before updating. The update starts when the backup finishes.',
			);
			expect(updateState.applying).toBe(true);
			expect(updateState.phase).toBe('watching');
		} finally {
			vi.useRealTimers();
		}
	});

	it('stops with the reason when the backup failed and the update was cancelled', async () => {
		vi.useFakeTimers();
		try {
			getSystemVersion.mockResolvedValueOnce(systemVersion());
			applyUpdate.mockResolvedValue({ triggered: true, backup_started: true, note: 'backup' });
			await updateState.startUpdate();

			const cancelled = systemVersion();
			cancelled.state.last_check_note = 'update cancelled: backup before update failed: no space';
			getSystemVersion.mockResolvedValue(cancelled);
			await vi.advanceTimersByTimeAsync(3_100);

			expect(toast.error).toHaveBeenCalledWith(
				'update cancelled: backup before update failed: no space',
			);
			expect(updateState.applying).toBe(false);
			expect(updateState.phase).toBe('idle');
		} finally {
			vi.useRealTimers();
		}
	});

	it('reloads only once the reported build version changes', async () => {
		vi.useFakeTimers();
		const reload = vi.fn();
		vi.stubGlobal('window', { location: { reload } });
		try {
			getSystemVersion.mockResolvedValueOnce(systemVersion());
			applyUpdate.mockResolvedValue({ triggered: true, note: 'scheduled' });
			await updateState.startUpdate();

			// The old container keeps answering with the same version while it pulls.
			getSystemVersion.mockResolvedValue(systemVersion());
			await vi.advanceTimersByTimeAsync(2_100);
			expect(reload).not.toHaveBeenCalled();

			// The replacement serves a different build → reload exactly once.
			const flipped = systemVersion();
			flipped.version = 'v0.1.22';
			getSystemVersion.mockResolvedValue(flipped);
			await vi.advanceTimersByTimeAsync(2_100);
			expect(reload).toHaveBeenCalledTimes(1);
		} finally {
			vi.unstubAllGlobals();
			vi.useRealTimers();
		}
	});
});
