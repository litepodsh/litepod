import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
	listApplicationTerminalSessions,
	listServerTerminalSessions,
	TerminalSocket,
} from './terminal-client';

class MockWebSocket {
	static instances: MockWebSocket[] = [];
	static readonly CONNECTING = 0;
	static readonly OPEN = 1;
	static readonly CLOSING = 2;
	static readonly CLOSED = 3;

	readonly url: string;
	readyState = MockWebSocket.CONNECTING;
	sent: { binary?: ArrayBufferLike; text?: string }[] = [];
	listeners: Record<string, Array<(ev: unknown) => void>> = {};
	closed = false;

	constructor(url: string) {
		this.url = url;
		MockWebSocket.instances.push(this);
	}

	send = vi.fn((data: string | ArrayBufferLike | Blob | ArrayBufferView) => {
		if (typeof data === 'string') {
			this.sent.push({ text: data });
		} else if (data instanceof ArrayBuffer) {
			this.sent.push({ binary: data });
		} else if (ArrayBuffer.isView(data)) {
			this.sent.push({ binary: data.buffer });
		}
	});

	close = vi.fn(() => {
		this.closed = true;
		this.readyState = MockWebSocket.CLOSED;
	});

	addEventListener(type: string, cb: (ev: unknown) => void) {
		(this.listeners[type] ??= []).push(cb);
	}
	removeEventListener(type: string, cb: (ev: unknown) => void) {
		const arr = this.listeners[type];
		if (!arr) return;
		const idx = arr.indexOf(cb);
		if (idx >= 0) arr.splice(idx, 1);
	}
	dispatch(type: string, ev: unknown) {
		for (const cb of this.listeners[type] ?? []) cb(ev);
	}
}

beforeEach(() => {
	MockWebSocket.instances = [];
	vi.stubGlobal('WebSocket', MockWebSocket as unknown as typeof WebSocket);
});

afterEach(() => {
	vi.unstubAllGlobals();
});

const lastSocket = () => MockWebSocket.instances[MockWebSocket.instances.length - 1];
const mockOf = (sock: TerminalSocket) => sock.socket as unknown as MockWebSocket;

const INPUT = {
	organizationId: 't1',
	projectId: 'p1',
	environmentId: 'e1',
	applicationId: 'a1',
	containerId: 'c1',
	execId: 'x1',
};

describe('TerminalSocket', () => {
	it('opens a WebSocket pointing at the terminal exec endpoint', () => {
		TerminalSocket.open(INPUT);
		const s = lastSocket();
		expect(s.url).toContain(
			'/api/v1/organizations/t1/projects/p1/environments/e1/applications/a1/containers/c1/exec/x1/ws',
		);
	});

	it('sends resize as a single text frame with trailing newline', () => {
		const sock = TerminalSocket.open(INPUT);
		sock.sendResize(120, 40);
		const m = mockOf(sock);
		expect(m.sent).toHaveLength(1);
		expect(m.sent[0].text).toBe('{"type":"resize","cols":120,"rows":40}\n');
	});

	it('sends stdin as a single binary frame with exact bytes', () => {
		const sock = TerminalSocket.open(INPUT);
		const bytes = new TextEncoder().encode('ls -la\n');
		sock.sendStdin(bytes);
		const m = mockOf(sock);
		expect(m.sent).toHaveLength(1);
		const view = new Uint8Array(m.sent[0].binary as ArrayBuffer);
		expect(Array.from(view)).toEqual(Array.from(bytes));
	});

	it('encodes string stdin as UTF-8 binary', () => {
		const sock = TerminalSocket.open(INPUT);
		sock.sendStdin('echo hi\n');
		const m = mockOf(sock);
		const view = new Uint8Array(m.sent[0].binary as ArrayBuffer);
		expect(new TextDecoder().decode(view)).toBe('echo hi\n');
	});

	it('dispatches onData only for binary messages from the server', () => {
		const sock = TerminalSocket.open(INPUT);
		const dataCb = vi.fn();
		sock.onData(dataCb);
		const m = mockOf(sock);
		const bytes = new Uint8Array([104, 105]);
		m.dispatch('message', { data: bytes.buffer });
		expect(dataCb).toHaveBeenCalledOnce();
		const arg = dataCb.mock.calls[0][0] as Uint8Array;
		expect(Array.from(arg)).toEqual([104, 105]);

		dataCb.mockClear();
		m.dispatch('message', { data: 'whatever' });
		expect(dataCb).not.toHaveBeenCalled();
	});

	it('dispatches onClose and stops emitting after close', () => {
		const sock = TerminalSocket.open(INPUT);
		const closeCb = vi.fn();
		const dataCb = vi.fn();
		sock.onClose(closeCb);
		sock.onData(dataCb);
		const m = mockOf(sock);
		m.readyState = MockWebSocket.OPEN;
		sock.close();
		m.dispatch('close', {});
		m.dispatch('message', { data: new Uint8Array([1]).buffer });
		expect(closeCb).toHaveBeenCalledOnce();
		expect(dataCb).not.toHaveBeenCalled();
	});

	it('onData and onClose return unsubscribe functions', () => {
		const sock = TerminalSocket.open(INPUT);
		const off1 = sock.onData(vi.fn());
		const off2 = sock.onClose(vi.fn());
		expect(() => off1()).not.toThrow();
		expect(() => off2()).not.toThrow();
	});

	it('parses server-side error text frames and dispatches onError', () => {
		const sock = TerminalSocket.open(INPUT);
		const errCb = vi.fn();
		sock.onError(errCb);
		const m = mockOf(sock);
		m.dispatch('message', { data: '{"type":"error","message":"bash not found"}' });
		expect(errCb).toHaveBeenCalledOnce();
		expect(errCb).toHaveBeenCalledWith('bash not found');
		expect(sock.hasReceivedError()).toBe(true);
	});

	it('ignores text frames that are not JSON control', () => {
		const sock = TerminalSocket.open(INPUT);
		const errCb = vi.fn();
		sock.onError(errCb);
		const m = mockOf(sock);
		m.dispatch('message', { data: 'not json' });
		expect(errCb).not.toHaveBeenCalled();
	});

	it('invokes onOpen when the underlying socket opens', () => {
		const sock = TerminalSocket.open(INPUT);
		const openCb = vi.fn();
		sock.onOpen(openCb);
		mockOf(sock).dispatch('open', {});
		expect(openCb).toHaveBeenCalledOnce();
	});
});

describe('terminal session history', () => {
	it('lists app and server sessions with limit and keyset cursor', async () => {
		const fetchMock = vi.fn(async () => new Response(JSON.stringify({ error: null, data: [] })));
		vi.stubGlobal('fetch', fetchMock);
		await listApplicationTerminalSessions(
			{ organizationId: 'o', projectId: 'p', environmentId: 'e', applicationId: 'a' },
			{ limit: 20, before: 'x' },
		);
		await listServerTerminalSessions('o', 's 1');
		const urls = fetchMock.mock.calls.map((call) => String((call as unknown[])[0]));
		expect(urls[0]).toMatch(
			/\/api\/v1\/organizations\/o\/projects\/p\/environments\/e\/applications\/a\/terminal-sessions\?limit=20&before=x$/,
		);
		expect(urls[1]).toMatch(/\/api\/v1\/organizations\/o\/servers\/s%201\/terminal-sessions$/);
		vi.unstubAllGlobals();
	});
});
