import { describe, expect, it, vi } from 'vitest';
import type { ServerGeo } from '#lib/clients/server-client.js';
import type { GlobePalette } from './server-globe/palette';
import { serverMarker } from './server-marker';

const palette: GlobePalette = {
	dark: true,
	atmosphere: '#445566',
	terminator: 0.3,
	accent: '#ff6600',
	ok: '#22aa55',
	warn: '#ddaa00',
	muted: '#888888',
};

const geo = (over: Partial<ServerGeo> = {}): ServerGeo => ({
	server_id: 's1',
	name: 'ash-1',
	kind: 'ssh',
	state: 'ready',
	ingress_mode: 'public_edge',
	is_main: false,
	latitude: 39,
	longitude: -77,
	city: 'Ashburn, Virginia',
	country: 'US',
	timezone: 'America/New_York',
	asn: null,
	unlocated_reason: null,
	...over,
});

const opts = { mainLabel: 'Control plane', stateLabel: 'Ready', time: '09:30', onSelect: () => {} };

describe('serverMarker', () => {
	it('hangs a state-coloured marker button from a zero-size anchor', () => {
		const anchor = serverMarker(geo(), palette, opts);
		expect(anchor.matches('div.gm-anchor')).toBe(true);
		const button = anchor.querySelector('button.gm') as HTMLButtonElement;
		expect(button).not.toBeNull();
		expect(button.classList.contains('st-ready')).toBe(true);
		expect(button.style.getPropertyValue('--c')).toBe(palette.ok);
		expect(button.textContent).toContain('ash-1');
		expect(button.textContent).toContain('Ashburn');
		expect(button.textContent).not.toContain('Virginia');
		expect(button.textContent).toContain('09:30');
	});

	it('marks the main server and paints it with the accent', () => {
		const button = serverMarker(geo({ is_main: true, state: 'degraded' }), palette, opts)
			.firstElementChild as HTMLElement;
		expect(button.classList.contains('main')).toBe(true);
		expect(button.classList.contains('st-degraded')).toBe(true);
		expect(button.style.getPropertyValue('--c')).toBe(palette.accent);
		expect(button.textContent).toContain('Control plane');
	});

	it('falls back to the country and treats the name as text', () => {
		const onSelect = vi.fn();
		const anchor = serverMarker(geo({ name: '<b>x</b>', city: null, state: 'offline' }), palette, {
			...opts,
			onSelect,
		});
		const button = anchor.querySelector('button.gm') as HTMLButtonElement;
		expect(button.querySelector('b b')).toBeNull();
		expect(button.textContent).toContain('<b>x</b>');
		expect(button.textContent).toContain('US');
		expect(button.style.getPropertyValue('--c')).toBe(palette.muted);
		button.click();
		expect(onSelect).toHaveBeenCalledOnce();
	});
});
