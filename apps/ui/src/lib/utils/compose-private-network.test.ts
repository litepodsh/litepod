import { describe, expect, it } from 'vitest';

import { makePrivateNetworkExplicit } from './compose-private-network';

describe('makePrivateNetworkExplicit', () => {
	it('adds an editable private network to every implicit service', () => {
		const result = makePrivateNetworkExplicit(
			'services:\n  app:\n    image: docmost\n  db:\n    image: postgres\nvolumes:\n  data: {}\n',
			'infisical-7vsns',
		);

		expect(result).toEqual({
			kind: 'updated',
			content:
				'services:\n  app:\n    image: docmost\n    networks:\n      private: {}\n  db:\n    image: postgres\n    networks:\n      private: {}\nvolumes:\n  data: {}\n\nnetworks:\n  private:\n    name: infisical-7vsns_private',
		});
	});

	it('leaves custom network topologies alone', () => {
		expect(
			makePrivateNetworkExplicit(
				'services:\n  app:\n    image: nginx\n    networks: [frontend]\n',
				'infisical-7vsns',
			),
		).toEqual({ kind: 'already-configured' });
	});

	it('strips trailing blank lines inside each service block before inserting', () => {
		const result = makePrivateNetworkExplicit(
			'services:\n  drawio:\n    image: drawio\n    environment:\n      FOO: bar\n      \nvolumes:\n  data: {}\n',
			'infisical-7vsns',
		);

		expect(result).toEqual({
			kind: 'updated',
			content:
				'services:\n  drawio:\n    image: drawio\n    environment:\n      FOO: bar\n    networks:\n      private: {}\nvolumes:\n  data: {}\n\nnetworks:\n  private:\n    name: infisical-7vsns_private',
		});
	});
});
