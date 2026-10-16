import { describe, expect, it } from 'vitest';
import { canOpenLocalConsole, deploymentTargetNotice, serverGroups } from './server-visibility';

const local = { id: 'local', kind: 'local' };
const remote = { id: 'remote', kind: 'ssh' };

describe('serverGroups', () => {
	it('hides the local server in cloud mode', () => {
		expect(serverGroups([local, remote], true)).toEqual({ local: [], remote: [remote] });
	});

	it('keeps the existing grouping in standard mode', () => {
		expect(serverGroups([local, remote], false)).toEqual({ local: [local], remote: [remote] });
	});
});

it('explains the cloud prerequisite when no ready target exists', () => {
	expect(deploymentTargetNotice(true, 0)).toContain('remote server');
});

describe('canOpenLocalConsole', () => {
	it('allows only a ready local server for a sysadmin', () => {
		expect(
			canOpenLocalConsole({
				isSysadmin: true,
				kind: 'local',
				state: 'ready',
			}),
		).toBe(true);
		expect(
			canOpenLocalConsole({
				isSysadmin: false,
				kind: 'local',
				state: 'ready',
			}),
		).toBe(false);
		expect(
			canOpenLocalConsole({
				isSysadmin: true,
				kind: 'ssh',
				state: 'ready',
			}),
		).toBe(false);
		expect(
			canOpenLocalConsole({
				isSysadmin: true,
				kind: 'local',
				state: 'offline',
			}),
		).toBe(false);
	});
});
