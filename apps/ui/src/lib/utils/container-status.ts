import type { StatusTone } from './status-tone';

export function isRunning(status: string): boolean {
	const normalized = status.toLowerCase();
	return (
		normalized === 'running' ||
		normalized === 'up' ||
		normalized.startsWith('running') ||
		normalized.startsWith('up')
	);
}

/**
 * Maps every container / application status string the app renders onto the
 * shared status vocabulary. Covers both Podman's own states (`running`,
 * `exited`, `created`, `configured`, `paused`) and litepod's deployment states
 * (`deploying`, `ready`, `not_deployed`, `failed`), because the same badge
 * shows up on the environment list, the containers tab and the Podman console.
 *
 * Anything unrecognised is `danger`, not `neutral`: an unknown container state
 * is a problem worth showing, and Podman reports `dead` / `removing` / `error`.
 */
export function containerStatusTone(status: string): StatusTone {
	const s = status.toLowerCase();
	if (isRunning(s)) return 'ok';
	switch (s) {
		case 'exited':
		case 'stopped':
		case 'down':
		case 'not_deployed':
		case 'unavailable':
		case 'not_found':
			return 'neutral';
		case 'created':
		case 'configured':
		case 'paused':
		case 'deploying':
		case 'restarting':
			return 'warn';
		case 'ready':
			return 'info';
		default:
			return 'danger';
	}
}
