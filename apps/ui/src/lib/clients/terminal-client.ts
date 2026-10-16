import { API_URL, api } from './base-client';

/** One row of terminal session history. Mirrors `TerminalSessionDto`. */
export type TerminalSessionDto = {
	id: string;
	kind: 'host' | 'container';
	container_id: string | null;
	/** SQLite `datetime('now')`: UTC `YYYY-MM-DD HH:MM:SS`. */
	started_at: string;
	finished_at: string | null;
	duration_seconds: number | null;
	result: 'closed' | 'disconnected' | 'failed' | null;
	failure_reason: string | null;
	actor_name: string | null;
	actor_email: string | null;
	actor_token_name: string | null;
};

export type TerminalSessionPage = { limit?: number; before?: string };

function pageQuery({ limit, before }: TerminalSessionPage): string {
	const params = new URLSearchParams();
	if (limit) params.set('limit', String(limit));
	if (before) params.set('before', before);
	const query = params.toString();
	return query ? `?${query}` : '';
}

export async function listApplicationTerminalSessions(
	ids: Pick<CreateExecInput, 'organizationId' | 'projectId' | 'environmentId' | 'applicationId'>,
	page: TerminalSessionPage = {},
): Promise<TerminalSessionDto[]> {
	const path = `/api/v1/organizations/${encodeURIComponent(ids.organizationId)}/projects/${encodeURIComponent(
		ids.projectId,
	)}/environments/${encodeURIComponent(ids.environmentId)}/applications/${encodeURIComponent(
		ids.applicationId,
	)}/terminal-sessions`;
	return (await api<TerminalSessionDto[]>(path + pageQuery(page))).data ?? [];
}

export async function listServerTerminalSessions(
	organizationId: string,
	serverId: string,
	page: TerminalSessionPage = {},
): Promise<TerminalSessionDto[]> {
	const path = `/api/v1/organizations/${encodeURIComponent(organizationId)}/servers/${encodeURIComponent(
		serverId,
	)}/terminal-sessions`;
	return (await api<TerminalSessionDto[]>(path + pageQuery(page))).data ?? [];
}

export type CreateExecInput = {
	organizationId: string;
	projectId: string;
	environmentId: string;
	applicationId: string;
	containerId: string;
	command: string[];
	tty: boolean;
};

export type CreateExecResponse = { execId: string };

export type OpenExecSocketInput = {
	organizationId: string;
	projectId: string;
	environmentId: string;
	applicationId: string;
	containerId: string;
	execId: string;
};

function execCreatePath(
	input: Pick<
		CreateExecInput,
		'organizationId' | 'projectId' | 'environmentId' | 'applicationId' | 'containerId'
	>,
): string {
	return `/api/v1/organizations/${encodeURIComponent(input.organizationId)}/projects/${encodeURIComponent(
		input.projectId,
	)}/environments/${encodeURIComponent(input.environmentId)}/applications/${encodeURIComponent(
		input.applicationId,
	)}/containers/${encodeURIComponent(input.containerId)}/exec`;
}

function execWsPath(input: OpenExecSocketInput): string {
	const base = execCreatePath(input);
	return `${base}/${encodeURIComponent(input.execId)}/ws`;
}

export async function createExec(input: CreateExecInput): Promise<CreateExecResponse> {
	const response = await fetch(`${API_URL}${execCreatePath(input)}`, {
		method: 'POST',
		credentials: 'include',
		headers: { 'content-type': 'application/json' },
		body: JSON.stringify({ command: input.command, tty: input.tty }),
	});
	if (!response.ok) {
		const text = await response.text();
		throw new Error(`createExec failed: ${response.status} ${text}`);
	}
	const data = (await response.json()) as { exec_id: string };
	return { execId: data.exec_id };
}

export function openExecSocket(input: OpenExecSocketInput): WebSocket {
	const base = API_URL.replace(/^http/, 'ws');
	return new WebSocket(`${base}${execWsPath(input)}`);
}

type Listener<T> = (value: T) => void;

export class TerminalSocket {
	readonly socket: WebSocket;
	private dataListeners = new Set<Listener<Uint8Array>>();
	private closeListeners = new Set<Listener<void>>();
	private openListeners = new Set<Listener<void>>();
	private errorListeners = new Set<Listener<string>>();
	private closed = false;
	private gotData = false;
	private gotError = false;

	constructor(socket: WebSocket) {
		this.socket = socket;
		this.socket.binaryType = 'arraybuffer';
		this.socket.addEventListener('message', this.handleMessage);
		this.socket.addEventListener('close', this.handleClose);
		this.socket.addEventListener('open', this.handleOpen);
	}

	static open(input: OpenExecSocketInput): TerminalSocket {
		return new TerminalSocket(openExecSocket(input));
	}

	private handleOpen = () => {
		this.openListeners.forEach((cb) => cb());
	};

	private handleMessage = (event: MessageEvent) => {
		if (this.closed) return;
		const data = event.data;
		if (data instanceof ArrayBuffer) {
			this.gotData = true;
			const copy = new Uint8Array(data.byteLength);
			copy.set(new Uint8Array(data));
			this.dataListeners.forEach((cb) => cb(copy));
		} else if (ArrayBuffer.isView(data)) {
			this.gotData = true;
			const view = data as ArrayBufferView;
			const copy = new Uint8Array(view.byteLength);
			copy.set(new Uint8Array(view.buffer, view.byteOffset, view.byteLength));
			this.dataListeners.forEach((cb) => cb(copy));
		} else if (typeof data === 'string') {
			// Server may send a JSON control error frame if exec.start fails.
			try {
				const parsed = JSON.parse(data) as { type?: string; message?: string };
				if (parsed?.type === 'error' && typeof parsed.message === 'string') {
					this.gotError = true;
					this.errorListeners.forEach((cb) => cb(parsed.message as string));
				}
			} catch {
				// Not JSON; ignore.
			}
		}
	};

	private handleClose = () => {
		this.closed = true;
		this.closeListeners.forEach((cb) => cb());
	};

	sendStdin(data: string | Uint8Array): void {
		if (this.closed) return;
		const bytes = typeof data === 'string' ? new TextEncoder().encode(data) : data;
		this.socket.send(bytes as unknown as BufferSource);
	}

	sendResize(cols: number, rows: number): void {
		if (this.closed) return;
		const text = `{"type":"resize","cols":${cols},"rows":${rows}}\n`;
		this.socket.send(text);
	}

	onData(cb: (bytes: Uint8Array) => void): () => void {
		this.dataListeners.add(cb);
		return () => this.dataListeners.delete(cb);
	}

	onClose(cb: () => void): () => void {
		this.closeListeners.add(cb);
		return () => this.closeListeners.delete(cb);
	}

	onOpen(cb: () => void): () => void {
		this.openListeners.add(cb);
		return () => this.openListeners.delete(cb);
	}

	onError(cb: (message: string) => void): () => void {
		this.errorListeners.add(cb);
		return () => this.errorListeners.delete(cb);
	}

	hasReceivedData(): boolean {
		return this.gotData;
	}

	hasReceivedError(): boolean {
		return this.gotError;
	}

	close(): void {
		if (this.closed) return;
		this.closed = true;
		this.socket.close();
	}

	get readyState(): number {
		return this.socket.readyState;
	}
}
