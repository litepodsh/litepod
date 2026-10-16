import type {
	DeploymentQueueEvent,
	DeploymentRunDetailDto,
	DeploymentRunLogDto,
	DeploymentRunStepDto,
} from '#lib/clients/project-client.js';

export type QueueEventContext = {
	activeJobId: string | null;
	livePaused: boolean;
	run: DeploymentRunDetailDto | null;
	now: number;
};

export type QueueEventAction =
	| { type: 'append'; log: DeploymentRunLogDto }
	| { type: 'status'; status: string }
	| { type: 'steps'; steps: DeploymentRunStepDto[] }
	| { type: 'terminal'; status: string }
	| { type: 'resync' }
	| { type: 'ignore' };

// Decides what a queue event means for the currently viewed run. Kept free of
// component state so both the log-dedup rules and the resync signal are
// testable without mounting the page.
export function queueEventAction(
	event: DeploymentQueueEvent,
	context: QueueEventContext,
): QueueEventAction {
	// A resync carries no job id: it only reports that this subscriber fell
	// behind, so it must be handled before filtering by job.
	if (event.kind === 'resync') {
		return context.livePaused ? { type: 'ignore' } : { type: 'resync' };
	}
	if (event.job_id !== context.activeJobId) return { type: 'ignore' };
	if (event.kind === 'status') return { type: 'status', status: event.payload };
	if (event.kind === 'terminal') return { type: 'terminal', status: event.payload };
	// Step changes are cheap and rare, so they apply even while the log is paused.
	if (event.kind === 'steps') {
		try {
			return { type: 'steps', steps: JSON.parse(event.payload) as DeploymentRunStepDto[] };
		} catch {
			return { type: 'ignore' };
		}
	}
	if (context.livePaused || event.sequence === null || !context.run) return { type: 'ignore' };
	if (context.run.logs.some((log) => log.sequence === event.sequence)) return { type: 'ignore' };
	return {
		type: 'append',
		log: {
			sequence: event.sequence,
			level: event.level ?? 'info',
			message: event.payload,
			created_at: context.now,
			server_id: event.server_id ?? null,
		},
	};
}
