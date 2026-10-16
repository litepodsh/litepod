import type { DeploymentRunActorDto } from '#lib/clients/project-client.js';

export function deploymentActionLabel(triggeredBy: string | null): string {
	switch (triggeredBy) {
		case 'manual_start':
			return 'Start';
		case 'manual_stop':
			return 'Stop';
		case 'manual_remove':
			return 'Remove';
		default:
			return 'Deploy';
	}
}

export function deploymentTriggerLabel(triggeredBy: string | null): string {
	if (triggeredBy === 'assistant_deploy') return 'AI assistant';
	return triggeredBy === 'webhook' || triggeredBy === 'deploy_hook' ? 'Automatic' : 'Manual';
}

export function deploymentActorLabel(
	actor: DeploymentRunActorDto | null,
	triggeredBy: string | null,
): string {
	// A run the assistant queued names the assistant first: the actor is the
	// person who confirmed it, which is true but not what the row is about.
	if (triggeredBy === 'assistant_deploy') {
		return actor ? `AI assistant · confirmed by ${actor.name}` : 'AI assistant';
	}
	if (actor) return `${actor.name} · ${actor.email}`;
	if (triggeredBy?.startsWith('manual_')) return 'Deleted user';
	return deploymentTriggerLabel(triggeredBy);
}
