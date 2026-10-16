import { describe, expect, it } from 'vitest';

import { createServerRequest, serverActionLabel } from './server-client';

describe('server registration presentation', () => {
	it('preserves the explicit ingress mode and generated-key choice', () => {
		expect(
			createServerRequest({
				name: 'edge',
				host: 'edge.example.test',
				port: 22,
				user: 'litepod',
				ingress_mode: 'public_edge',
				podman_mode: 'rootless',
				credential: { kind: 'generate' },
			}),
		).toMatchObject({ ingress_mode: 'public_edge', credential: { kind: 'generate' } });
	});

	it('offers the safe next action for each recoverable state', () => {
		expect(serverActionLabel({ state: 'needs_setup' })).toBe('Set up server');
		expect(serverActionLabel({ state: 'degraded' })).toBe('Repair server');
	});
});
