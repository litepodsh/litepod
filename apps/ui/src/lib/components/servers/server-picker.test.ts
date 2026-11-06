import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, expect, it } from 'vitest';
import ServerPicker from './server-picker.svelte';

// jsdom has no layout, so the command list's scroll-into-view has nothing to call.
Element.prototype.scrollIntoView = () => {};

afterEach(cleanup);

const servers = [
	{ id: 'local', name: 'This machine', os: 'macos', distribution: null, ingress_mode: 'central' },
	{
		id: 'edge',
		name: 'Edge in Paris',
		os: null,
		distribution: 'ubuntu',
		ingress_mode: 'public_edge',
	},
];

function renderPicker(value = '') {
	return render(ServerPicker, {
		servers,
		value,
		placeholder: 'Choose a server',
		searchPlaceholder: 'Search servers…',
		emptyText: 'No servers match this search.',
	});
}

it('shows the preselected server with its ingress mode', () => {
	renderPicker('edge');
	const trigger = screen.getByRole('combobox');
	expect(trigger.textContent).toContain('Edge in Paris');
	expect(trigger.textContent).toContain('public edge');
});

it('searches servers by name and picks one', async () => {
	renderPicker('local');
	const trigger = screen.getByRole('combobox');
	expect(trigger.textContent).toContain('This machine');

	await fireEvent.click(trigger);
	const input = screen.getByPlaceholderText('Search servers…');
	await fireEvent.input(input, { target: { value: 'paris' } });

	const option = await screen.findByRole('option', { name: /Edge in Paris/ });
	await fireEvent.click(option);

	expect(trigger.textContent).toContain('Edge in Paris');
	expect(trigger.textContent).toContain('public edge');
});
