import { describe, expect, it } from 'vitest';
import {
	deploymentActionLabel,
	deploymentActorLabel,
	deploymentTriggerLabel,
} from './deployment-run-presentation';

describe('deployment run presentation', () => {
	it.each([
		['manual', 'Deploy'],
		['manual_deploy', 'Deploy'],
		['manual_start', 'Start'],
		['manual_stop', 'Stop'],
		['manual_remove', 'Remove'],
		['webhook', 'Deploy'],
		['deploy_hook', 'Deploy'],
	] as const)('maps %s to %s', (trigger, label) => {
		expect(deploymentActionLabel(trigger)).toBe(label);
	});

	it('labels automatic and manual triggers', () => {
		expect(deploymentTriggerLabel('webhook')).toBe('Automatic');
		expect(deploymentTriggerLabel('deploy_hook')).toBe('Automatic');
		expect(deploymentTriggerLabel('manual_start')).toBe('Manual');
	});

	it('shows the actor or a deleted-user fallback for audited manual actions', () => {
		expect(
			deploymentActorLabel({ id: 'user-1', name: 'Ada', email: 'ada@example.com' }, 'manual_stop'),
		).toBe('Ada · ada@example.com');
		expect(deploymentActorLabel(null, 'manual_remove')).toBe('Deleted user');
		expect(deploymentActorLabel(null, 'webhook')).toBe('Automatic');
		expect(deploymentActorLabel(null, 'manual')).toBe('Manual');
	});
});

it('names the assistant when it queued the run', () => {
	expect(deploymentTriggerLabel('assistant_deploy')).toBe('AI assistant');
});

it('credits the assistant first, and the person who confirmed it second', () => {
	const actor = { id: 'u1', name: 'Ada', email: 'ada@example.com' };
	expect(deploymentActorLabel(actor, 'assistant_deploy')).toBe('AI assistant · confirmed by Ada');
	expect(deploymentActorLabel(null, 'assistant_deploy')).toBe('AI assistant');
});
