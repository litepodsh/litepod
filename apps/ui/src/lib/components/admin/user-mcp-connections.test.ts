import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, expect, it, vi } from 'vitest';

const list = vi.fn();
const revoke = vi.fn().mockResolvedValue(undefined);
vi.mock('#lib/clients/admin-users-client.js', () => ({
	listUserMcpConnections: (...args: unknown[]) => list(...args),
	revokeUserMcpConnection: (...args: unknown[]) => revoke(...args),
}));

import UserMcpConnections from './user-mcp-connections.svelte';

const connection = (id: string, closed_at: string | null, expires_at = '2999-01-01 00:00:00') => ({
	id,
	client_name: id,
	agent: 'claude',
	organization_name: 'Acme',
	created_at: '2026-01-01 00:00:00',
	expires_at,
	closed_at,
	last_used_at: null,
});

afterEach(() => {
	cleanup();
	vi.clearAllMocks();
	vi.unstubAllGlobals();
});

it('offers to disconnect only the open connections, not the closed or lapsed ones', async () => {
	list.mockResolvedValue([
		connection('open', null),
		connection('closed', '2026-02-01 00:00:00'),
		connection('lapsed', null, '2026-02-01 00:00:00'),
	]);
	render(UserMcpConnections, { userId: 'user-1' });

	await screen.findByText('lapsed');
	expect(list).toHaveBeenCalledWith('user-1');
	expect(screen.getAllByRole('button')).toHaveLength(1);
});

it('disconnects after confirming and reloads, and does nothing when declined', async () => {
	list.mockResolvedValue([connection('open', null)]);
	render(UserMcpConnections, { userId: 'user-1' });
	const button = await screen.findByRole('button');

	vi.stubGlobal('confirm', () => false);
	await fireEvent.click(button);
	expect(revoke).not.toHaveBeenCalled();

	vi.stubGlobal('confirm', () => true);
	list.mockResolvedValue([connection('open', '2026-02-01 00:00:00')]);
	await fireEvent.click(button);
	await waitFor(() => expect(revoke).toHaveBeenCalledWith('user-1', 'open'));
	await waitFor(() => expect(screen.queryByRole('button')).toBeNull());
});
