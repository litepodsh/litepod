import { describe, expect, it } from 'vitest';
import { queueEventAction } from './deployment-queue-events';
import type { DeploymentQueueEvent, DeploymentRunDetailDto } from '#lib/clients/project-client.js';

const run = {
	id: 'run-1',
	logs: [{ sequence: 0, level: 'info', message: 'Deployment started', created_at: 1 }],
} as unknown as DeploymentRunDetailDto;

const event = (overrides: Partial<DeploymentQueueEvent>): DeploymentQueueEvent => ({
	job_id: 'job-1',
	kind: 'log',
	sequence: 1,
	level: 'info',
	payload: 'Cloning repository',
	...overrides,
});

describe('queueEventAction', () => {
	it('appends a log line for the active job', () => {
		const action = queueEventAction(event({}), {
			activeJobId: 'job-1',
			livePaused: false,
			run,
			now: 42,
		});
		expect(action).toEqual({
			type: 'append',
			log: {
				sequence: 1,
				level: 'info',
				message: 'Cloning repository',
				created_at: 42,
				server_id: null,
			},
		});
	});

	it('ignores a log line already in the run', () => {
		const action = queueEventAction(event({ sequence: 0 }), {
			activeJobId: 'job-1',
			livePaused: false,
			run,
			now: 42,
		});
		expect(action).toEqual({ type: 'ignore' });
	});

	it('ignores events from another job', () => {
		const action = queueEventAction(event({ job_id: 'job-2' }), {
			activeJobId: 'job-1',
			livePaused: false,
			run,
			now: 42,
		});
		expect(action).toEqual({ type: 'ignore' });
	});

	it('ignores log lines while live output is paused', () => {
		const action = queueEventAction(event({}), {
			activeJobId: 'job-1',
			livePaused: true,
			run,
			now: 42,
		});
		expect(action).toEqual({ type: 'ignore' });
	});

	it('asks for a resync regardless of job id', () => {
		const action = queueEventAction(
			event({ job_id: '', kind: 'resync', sequence: null, level: null, payload: '17' }),
			{ activeJobId: 'job-1', livePaused: false, run, now: 42 },
		);
		expect(action).toEqual({ type: 'resync' });
	});

	it('does not resync while live output is paused', () => {
		const action = queueEventAction(
			event({ job_id: '', kind: 'resync', sequence: null, level: null, payload: '17' }),
			{ activeJobId: 'job-1', livePaused: true, run, now: 42 },
		);
		expect(action).toEqual({ type: 'ignore' });
	});

	it('reports status and terminal events for the active job', () => {
		const context = { activeJobId: 'job-1', livePaused: false, run, now: 42 };
		expect(queueEventAction(event({ kind: 'status', payload: 'building' }), context)).toEqual({
			type: 'status',
			status: 'building',
		});
		expect(queueEventAction(event({ kind: 'terminal', payload: 'succeeded' }), context)).toEqual({
			type: 'terminal',
			status: 'succeeded',
		});
	});

	it('applies step lists even while the log is paused', () => {
		const steps = [
			{ name: 'pull', status: 'running', started_at: 1, finished_at: null, server_id: null },
		];
		const context = { activeJobId: 'job-1', livePaused: true, run, now: 42 };
		expect(
			queueEventAction(
				event({ kind: 'steps', sequence: null, payload: JSON.stringify(steps) }),
				context,
			),
		).toEqual({ type: 'steps', steps });
		expect(
			queueEventAction(event({ kind: 'steps', sequence: null, payload: 'nope' }), context),
		).toEqual({ type: 'ignore' });
	});
});
