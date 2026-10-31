import { describe, expect, it, vi } from 'vitest';

import { createDeferredTabLoader } from './application-tab-loaders';

describe('deferred application tab loader', () => {
	it('loads a tab only once until reset', async () => {
		const deployment = vi.fn().mockResolvedValue(undefined);
		const loader = createDeferredTabLoader({
			containers: vi.fn(),
			deployment,
			domains: vi.fn(),
			environment: vi.fn(),
			networks: vi.fn(),
			volumes: vi.fn(),
		});

		await loader.load('deployment');
		await loader.load('deployment');

		expect(deployment).toHaveBeenCalledTimes(1);
	});

	it('shares an in-progress load for the same tab', async () => {
		let resolve!: () => void;
		const domains = vi.fn(() => new Promise<void>((done) => (resolve = done)));
		const loader = createDeferredTabLoader({
			containers: vi.fn(),
			deployment: vi.fn(),
			domains,
			environment: vi.fn(),
			networks: vi.fn(),
			volumes: vi.fn(),
		});

		const first = loader.load('domains');
		const second = loader.load('domains');
		resolve();
		await Promise.all([first, second]);

		expect(domains).toHaveBeenCalledTimes(1);
	});

	it('refreshes a loaded tab and reset permits loading it again', async () => {
		const environment = vi.fn().mockResolvedValue(undefined);
		const loader = createDeferredTabLoader({
			containers: vi.fn(),
			deployment: vi.fn(),
			domains: vi.fn(),
			environment,
			networks: vi.fn(),
			volumes: vi.fn(),
		});

		await loader.load('environment');
		await loader.refresh('environment');
		loader.reset();
		await loader.load('environment');

		expect(environment).toHaveBeenCalledTimes(3);
	});
});
