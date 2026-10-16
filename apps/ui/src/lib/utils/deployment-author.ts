import type { DeploymentRunActorDto } from '#lib/clients/project-client.js';
import { initials } from '#lib/utils/avatar.js';

export type DeploymentAuthor = {
	kind: 'user' | 'automatic' | 'deleted' | 'assistant';
	label: string;
	seed: string | null;
	initials: string;
};

function lower(value: string): string {
	return value.toLocaleLowerCase();
}

function nonEmpty(value: string | null): string | null {
	const trimmed = value?.trim();
	return trimmed ? trimmed : null;
}

export function deploymentAuthor(
	actor: DeploymentRunActorDto | null,
	triggeredBy: string | null,
	commitAuthorName: string | null,
	commitAuthorEmail: string | null,
): DeploymentAuthor {
	// The assistant proposed it and a person confirmed it. The row credits the
	// assistant, with the confirming user beside it when there is one.
	if (triggeredBy === 'assistant_deploy') {
		return {
			kind: 'assistant',
			label: actor ? `ai assistant · confirmed by ${lower(actor.name)}` : 'ai assistant',
			seed: null,
			initials: '',
		};
	}

	const isManual = triggeredBy === 'manual' || triggeredBy?.startsWith('manual_');
	if (isManual) {
		if (!actor) return { kind: 'deleted', label: 'deleted user', seed: null, initials: '' };
		const label = [actor.name, actor.email].filter(Boolean).map(lower).join(' · ');
		return {
			kind: 'user',
			label,
			seed: actor.id,
			initials: initials(actor.name || actor.email),
		};
	}

	const name = nonEmpty(commitAuthorName);
	const email = nonEmpty(commitAuthorEmail);
	if (name || email) {
		const label = [name, email]
			.filter((value): value is string => value !== null)
			.map(lower)
			.join(' · ');
		return {
			kind: 'user',
			label,
			seed: email ?? name,
			initials: initials(name ?? email ?? ''),
		};
	}

	return { kind: 'automatic', label: 'automatic', seed: null, initials: '' };
}
