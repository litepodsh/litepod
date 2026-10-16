import { API_URL } from './base-client';
import { TerminalSocket } from './terminal-client';

export type ServerTerminalShell = 'sh' | 'bash' | 'zsh' | 'nu' | 'fish' | 'pwsh';

export const SERVER_TERMINAL_SHELLS: readonly ServerTerminalShell[] = [
	'bash',
	'sh',
	'zsh',
	'nu',
	'fish',
	'pwsh',
];

export function openServerTerminalSocket(
	organizationId: string,
	serverId: string,
	// Bare name only — the API resolves it to an absolute path from a fixed
	// allowlist, so no executable path is ever sent over the wire.
	shell: ServerTerminalShell = 'bash',
): TerminalSocket {
	const base = API_URL.replace(/^http/, 'ws');
	const url = `${base}/api/v1/organizations/${encodeURIComponent(organizationId)}/servers/${encodeURIComponent(serverId)}/terminal/ws?shell=${encodeURIComponent(shell)}`;
	return new TerminalSocket(new WebSocket(url));
}
