import { describe, expect, it } from 'vitest';
import { deploymentAuthor } from './deployment-author';

describe('deployment author', () => {
	it('uses the litepod actor for manual deployments and normalizes its label', () => {
		expect(
			deploymentAuthor(
				{ id: 'user-42', name: 'Ada Admin', email: 'ADA@EXAMPLE.COM' },
				'manual_deploy',
				null,
				null,
			),
		).toMatchObject({
			kind: 'user',
			label: 'ada admin · ada@example.com',
			seed: 'user-42',
		});
	});

	it('uses the Git author for webhook deployments', () => {
		expect(deploymentAuthor(null, 'webhook', 'Linus Torvalds', 'LINUS@EXAMPLE.COM')).toMatchObject({
			kind: 'user',
			label: 'linus torvalds · linus@example.com',
			seed: 'LINUS@EXAMPLE.COM',
		});
	});

	it('uses the Git author name as the seed when the commit has no email', () => {
		expect(deploymentAuthor(null, 'deploy_hook', 'Grace Hopper', null)).toMatchObject({
			kind: 'user',
			label: 'grace hopper',
			seed: 'Grace Hopper',
		});
	});

	it('keeps meaningful fallbacks when identity data is unavailable', () => {
		expect(deploymentAuthor(null, 'manual_remove', null, null)).toMatchObject({
			kind: 'deleted',
			label: 'deleted user',
			seed: null,
		});
		expect(deploymentAuthor(null, 'deploy_hook', null, null)).toMatchObject({
			kind: 'automatic',
			label: 'automatic',
			seed: null,
		});
	});
});

it('marks an assistant run as its own kind, not as a user', () => {
	const actor = { id: 'u1', name: 'Ada', email: 'ada@example.com' };
	expect(deploymentAuthor(actor, 'assistant_deploy', null, null)).toMatchObject({
		kind: 'assistant',
		label: 'ai assistant · confirmed by ada',
		seed: null,
	});
	expect(deploymentAuthor(null, 'assistant_deploy', null, null)).toMatchObject({
		kind: 'assistant',
		label: 'ai assistant',
	});
});
