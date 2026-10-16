import { describe, expect, it } from 'vitest';

import { containerStatusTone, isRunning } from './container-status';

describe('container status', () => {
	it('recognises the running variants Podman and Compose each report', () => {
		expect(isRunning('running')).toBe(true);
		expect(isRunning('Up 4 minutes')).toBe(true);
		expect(isRunning('exited')).toBe(false);
	});

	it('maps podman and deployment states onto shared tones', () => {
		expect(containerStatusTone('Up 2 hours')).toBe('ok');
		expect(containerStatusTone('exited')).toBe('neutral');
		expect(containerStatusTone('not_deployed')).toBe('neutral');
		expect(containerStatusTone('paused')).toBe('warn');
		expect(containerStatusTone('deploying')).toBe('warn');
		expect(containerStatusTone('ready')).toBe('info');
		expect(containerStatusTone('failed')).toBe('danger');
	});

	it('treats an unknown state as danger rather than quietly neutral', () => {
		expect(containerStatusTone('dead')).toBe('danger');
		expect(containerStatusTone('something-new')).toBe('danger');
	});
});
