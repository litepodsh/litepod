import { describe, expect, it } from 'vitest';

import { findReservedServices } from './compose-reserved-services';

const names = (content: string) => findReservedServices(content).map((service) => service.name);

describe('findReservedServices', () => {
	it('flags a service that shadows a control-plane container', () => {
		const content = 'services:\n  libsql:\n    image: x\n  web:\n    image: y\n';
		expect(names(content)).toEqual(['libsql']);
	});

	it('reports 1-based positions covering the service name', () => {
		const [service] = findReservedServices('services:\n  app:\n    image: x\n');
		expect(service).toMatchObject({ name: 'app', line: 2, column: 3, length: 3 });
	});

	it('matches regardless of case and reports each name once', () => {
		const content = 'services:\n  Caddy:\n    image: x\n  caddy:\n    image: y\n';
		expect(names(content)).toEqual(['Caddy']);
	});

	it('ignores keys inside a service body', () => {
		const content = 'services:\n  web:\n    image: x\n    api:\n      enabled: true\n';
		expect(names(content)).toEqual([]);
	});

	it('stops at the end of the services block', () => {
		const content = 'services:\n  web:\n    image: x\nvolumes:\n  app:\n';
		expect(names(content)).toEqual([]);
	});

	it('skips comments and leaves safe names alone', () => {
		const content = 'services:\n  # libsql:\n  libsql-db:\n    image: x\n';
		expect(names(content)).toEqual([]);
	});
});
