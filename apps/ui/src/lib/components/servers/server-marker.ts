/** The glass pin globe.gl hangs over each server. Plain DOM: globe.gl owns these nodes, not Svelte. */

import type { ServerGeo } from '#lib/clients/server-client.js';
import { markerColor, type GlobePalette } from './server-globe/palette';

// Lucide's server, cloud and house glyphs, inlined because the marker is built outside Svelte.
const ICONS = {
	server:
		'<rect width="20" height="8" x="2" y="2" rx="2"/><rect width="20" height="8" x="2" y="14" rx="2"/><line x1="6" x2="6.01" y1="6" y2="6"/><line x1="6" x2="6.01" y1="18" y2="18"/>',
	cloud: '<path d="M17.5 19H9a7 7 0 1 1 6.71-9h1.79a4.5 4.5 0 1 1 0 9Z"/>',
	house:
		'<path d="M15 21v-8a1 1 0 0 0-1-1h-4a1 1 0 0 0-1 1v8"/><path d="M3 10a2 2 0 0 1 .709-1.528l7-6a2 2 0 0 1 2.582 0l7 6A2 2 0 0 1 21 10v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>',
};

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string) {
	const node = document.createElement(tag);
	node.className = className;
	if (text !== undefined) node.textContent = text;
	return node;
}

export function serverMarker(
	server: ServerGeo,
	palette: GlobePalette,
	{
		mainLabel,
		stateLabel,
		time,
		cloud = false,
		onSelect,
	}: {
		/** Subtitle for the main server, in place of its city. */
		mainLabel: string;
		stateLabel: string;
		/** Server's wall clock, or null when its zone is unknown. */
		time: string | null;
		cloud?: boolean;
		onSelect: () => void;
	},
): HTMLDivElement {
	const button = el('button', `gm st-${server.state}${server.is_main ? ' main' : ''}`);
	button.type = 'button';
	button.style.setProperty('--c', markerColor(server, palette));
	button.addEventListener('click', onSelect);

	const card = el('span', 'gm-card');
	const icon = el('span', 'gm-ico');
	const glyph = server.is_main ? (cloud ? 'cloud' : 'house') : 'server';
	icon.innerHTML = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${ICONS[glyph]}</svg>`;

	const text = el('span', 'gm-t');
	const place = server.is_main ? mainLabel : (server.city?.split(',')[0] ?? server.country ?? '');
	text.append(el('b', '', server.name), el('small', '', place));

	const more = el('span', 'gm-more');
	more.append(el('span', '', stateLabel), el('span', 'gm-time', time ?? ''));

	card.append(icon, text, more);
	button.append(card, el('span', 'gm-stem'));

	// globe.gl owns the outer element's transform, so the marker hangs from a zero-size anchor.
	const anchor = el('div', 'gm-anchor');
	anchor.append(button);
	return anchor;
}
