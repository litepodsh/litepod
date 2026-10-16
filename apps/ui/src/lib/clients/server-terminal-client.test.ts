import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { openServerTerminalSocket } from './server-terminal-client';

class MockWebSocket {
	static instances: MockWebSocket[] = [];
	static readonly CONNECTING = 0;
	static readonly OPEN = 1;
	static readonly CLOSED = 3;
	readonly url: string;
	readyState = MockWebSocket.CONNECTING;
	binaryType = '';
	listeners: Record<string, Array<(event: unknown) => void>> = {};

	constructor(url: string) {
		this.url = url;
		MockWebSocket.instances.push(this);
	}

	send = vi.fn();
	close = vi.fn(() => {
		this.readyState = MockWebSocket.CLOSED;
	});
	addEventListener(type: string, callback: (event: unknown) => void) {
		(this.listeners[type] ??= []).push(callback);
	}
	removeEventListener(type: string, callback: (event: unknown) => void) {
		this.listeners[type] = (this.listeners[type] ?? []).filter((listener) => listener !== callback);
	}
}

beforeEach(() => {
	MockWebSocket.instances = [];
	vi.stubGlobal('WebSocket', MockWebSocket as unknown as typeof WebSocket);
});

afterEach(() => vi.unstubAllGlobals());

describe('openServerTerminalSocket', () => {
	it('opens the scoped SSH terminal endpoint with the bare shell name', () => {
		const socket = openServerTerminalSocket('org/a', 'server b', 'bash');
		const native = socket.socket as unknown as MockWebSocket;
		expect(native.url).toContain('/organizations/org%2Fa/servers/server%20b/terminal/ws');
		expect(native.url).toContain('shell=bash');
		// The executable path is never sent — the API resolves the name.
		expect(native.url).not.toContain('%2Fbin%2F');
		expect(native.binaryType).toBe('arraybuffer');
	});

	it('uses bash by default so line history works', () => {
		openServerTerminalSocket('org', 'server');
		expect(MockWebSocket.instances[0].url).toContain('shell=bash');
	});
});
