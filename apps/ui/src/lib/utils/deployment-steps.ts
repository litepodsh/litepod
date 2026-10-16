import type { DeploymentRunStepDto } from '#lib/clients/project-client.js';
import { deploymentSteps } from '#lib/preview/preview-data.js';

export type StepState = 'done' | 'run' | 'todo' | 'failed' | 'skipped';
/** `name` is the API step key (`clone`, `health_check`, ...), translated by the viewer. */
export type DeploymentStep = { name: string; state: StepState; durationMs?: number | null };
export type StepKind = 'compose' | 'git' | 'image';

const ACTIVE = ['queued', 'building', 'deploying', 'cancel_requested'];

const STATE: Record<DeploymentRunStepDto['status'], StepState> = {
	pending: 'todo',
	running: 'run',
	succeeded: 'done',
	failed: 'failed',
	skipped: 'skipped',
};

/** Maps a run's `source_type` onto the pipeline shape the fallback step bar draws. */
export function stepKind(sourceType: string | null | undefined): StepKind {
	if (sourceType === 'git' || sourceType === 'github') return 'git';
	if (sourceType === 'image') return 'image';
	return 'compose';
}

/** The recorded steps of a single-server run (replica steps are left to the replica view). */
export function stepsFromRun(steps: DeploymentRunStepDto[] | undefined): DeploymentStep[] {
	return (steps ?? [])
		.filter((step) => !step.server_id)
		.map((step) => ({
			name: step.name,
			state: STATE[step.status] ?? 'todo',
			durationMs:
				step.started_at !== null && step.finished_at !== null
					? step.finished_at - step.started_at
					: null,
		}));
}

/**
 * Fallback for runs recorded before the API kept steps: the example step names
 * with every step done for a succeeded run, the first one running for an active
 * run, and nothing claimed for a failed or canceled run.
 */
export function stepsFromLog(_lines: string[], kind: StepKind, status: string): DeploymentStep[] {
	return deploymentSteps(kind).map((name, index) => ({
		name: name === 'Health' ? 'health_check' : name.toLowerCase(),
		state:
			status === 'succeeded' ? 'done' : ACTIVE.includes(status) && index === 0 ? 'run' : 'todo',
	}));
}

const LIFECYCLE = ['manual_start', 'manual_stop', 'manual_remove'];

/**
 * Real steps when the run has them. A live run without steps yet (still queued)
 * and a start/stop/remove action draw no bar; only finished runs recorded
 * before steps existed get the example fallback, flagged as preview.
 */
export function runSteps(
	run: { steps?: DeploymentRunStepDto[]; source_type: string; triggered_by?: string | null },
	status: string,
): { steps: DeploymentStep[]; preview: boolean } {
	const real = stepsFromRun(run.steps);
	if (real.length > 0) return { steps: real, preview: false };
	if (ACTIVE.includes(status) || LIFECYCLE.includes(run.triggered_by ?? '')) {
		return { steps: [], preview: false };
	}
	return { steps: stepsFromLog([], stepKind(run.source_type), status), preview: true };
}
