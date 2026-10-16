import { describe, expect, it } from 'vitest';
import type { DeploymentRunStepDto } from '#lib/clients/project-client.js';
import { runSteps, stepKind, stepsFromLog, stepsFromRun } from './deployment-steps';

const states = (status: string, kind = stepKind('compose')) =>
	stepsFromLog(['pull web'], kind, status).map((step) => step.state);

const step = (
	name: string,
	status: DeploymentRunStepDto['status'],
	extra: Partial<DeploymentRunStepDto> = {},
): DeploymentRunStepDto => ({
	name,
	status,
	started_at: null,
	finished_at: null,
	server_id: null,
	...extra,
});

describe('stepsFromLog', () => {
	it('marks every step done for a finished run', () => {
		expect(states('succeeded')).toEqual(['done', 'done', 'done', 'done', 'done']);
	});

	it('runs the first unfinished step of an active run', () => {
		expect(states('building')).toEqual(['run', 'todo', 'todo', 'todo', 'todo']);
		expect(states('queued')[0]).toBe('run');
	});

	it('claims no progress for a failed or canceled run', () => {
		expect(states('failed')).toEqual(['todo', 'todo', 'todo', 'todo', 'todo']);
		expect(states('canceled').every((s) => s === 'todo')).toBe(true);
	});

	it('uses step keys of the matching pipeline', () => {
		expect(stepsFromLog([], stepKind('github'), 'succeeded')[0].name).toBe('clone');
		expect(stepsFromLog([], stepKind('image'), 'succeeded').map((s) => s.name)).toEqual([
			'prepare',
			'pull',
			'start',
			'health_check',
			'route',
		]);
	});
});

describe('stepsFromRun', () => {
	it('maps statuses and durations and leaves replica steps out', () => {
		expect(
			stepsFromRun([
				step('pull', 'succeeded', { started_at: 10, finished_at: 250 }),
				step('start', 'running', { started_at: 250 }),
				step('route', 'skipped'),
				step('pull', 'failed', { server_id: 'server-a' }),
			]),
		).toEqual([
			{ name: 'pull', state: 'done', durationMs: 240 },
			{ name: 'start', state: 'run', durationMs: null },
			{ name: 'route', state: 'skipped', durationMs: null },
		]);
	});
});

describe('runSteps', () => {
	it('prefers recorded steps and only flags the fallback as preview', () => {
		const real = runSteps({ source_type: 'compose', steps: [step('prepare', 'failed')] }, 'failed');
		expect(real).toEqual({
			steps: [{ name: 'prepare', state: 'failed', durationMs: null }],
			preview: false,
		});
		const old = runSteps({ source_type: 'compose', steps: [] }, 'succeeded');
		expect(old.preview).toBe(true);
		expect(old.steps).toHaveLength(5);
	});

	it('draws no example bar for a live run or a lifecycle action', () => {
		expect(runSteps({ source_type: 'compose', steps: [] }, 'deploying')).toEqual({
			steps: [],
			preview: false,
		});
		expect(
			runSteps({ source_type: 'compose', steps: [], triggered_by: 'manual_stop' }, 'succeeded')
				.steps,
		).toEqual([]);
	});
});
