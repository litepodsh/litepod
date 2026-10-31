import { describe, expect, it } from 'vitest';

import {
	inspectedContainerPorts,
	inspectedIps,
	internalRoute,
	listedContainerPorts,
	splitImage,
} from './container-routes';

describe('container routes', () => {
	it('lists unique tcp container ports, sorted, ignoring udp', () => {
		const entry = {
			Ports: [
				{ host_port: 8080, container_port: 80, protocol: 'tcp' },
				{ host_port: 8443, container_port: 80, protocol: 'tcp' },
				{ host_port: 53, container_port: 53, protocol: 'udp' },
				{ container_port: 9000, protocol: 'tcp' },
			],
		};
		expect(listedContainerPorts(entry)).toEqual([80, 9000]);
		expect(listedContainerPorts({})).toEqual([]);
	});

	it('merges exposed and published ports from inspect', () => {
		const data = {
			Config: { ExposedPorts: { '3000/tcp': {}, '53/udp': {} } },
			NetworkSettings: { Ports: { '80/tcp': [{ HostPort: '8080' }] } },
		};
		expect(inspectedContainerPorts(data)).toEqual([80, 3000]);
	});

	it('builds the route and reads per-network IPs', () => {
		expect(internalRoute('web', 80)).toBe('http://web:80');
		const data = {
			NetworkSettings: {
				Networks: { app: { IPAddress: '10.89.0.4' }, host: { IPAddress: '' } },
			},
		};
		expect(inspectedIps(data)).toEqual([{ network: 'app', ip: '10.89.0.4' }]);
	});

	it('splits image repo from tag without breaking registry ports', () => {
		expect(splitImage('docker.io/library/nginx:1.27')).toEqual({
			repo: 'docker.io/library/nginx',
			tag: '1.27',
		});
		expect(splitImage('localhost:5000/app')).toEqual({
			repo: 'localhost:5000/app',
			tag: 'latest',
		});
		expect(splitImage('')).toEqual({ repo: '', tag: '' });
	});
});
