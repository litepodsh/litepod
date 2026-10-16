import { API_URL, api } from './base-client';

/**
 * Three endpoints for the whole assistant. What it can do is a tool list on the
 * API, not a route table, so a new capability never adds a call here.
 */

/**
 * A change the assistant wants to make. Nothing is applied until `settleAction`
 * confirms it, which is a separate authorised request.
 */
export type ActionDto = {
	id: string;
	tool: string;
	title: string;
	summary: string;
	status: 'pending' | 'applied' | 'rejected' | 'expired' | 'failed';
	/** Where in the app the change happens, so the panel can take the user there. */
	route: string;
	/** The application the change targets. Lets the panel broadcast a
	 * `application-updated` event after apply so any editor already on the
	 * application view refreshes without a manual reload. Empty for the
	 * project-scoped actions, which have no application. */
	application_id: string;
	/** The line the change adds, with the value masked. */
	added: string | null;
	replaces: string | null;
	/** The name the change removes. */
	removed: string | null;
	/** A truncated unified diff for line-based changes (compose). Absent for
	 * tools whose card already shows enough detail (env vars, redeploy). */
	diff?: string | null;
	expires_at: number;
	failure: string | null;
};

/**
 * What the API emits while a turn runs. The panel is a reducer over these, so a
 * tool that takes two seconds is visible for those two seconds instead of
 * appearing at the end with everything else.
 */
export type TurnEvent =
	| { type: 'conversation'; id: string }
	| { type: 'text'; text: string }
	| { type: 'tool_started'; id: string; name: string; arguments: string }
	| { type: 'tool_finished'; id: string; ok: boolean }
	| { type: 'action'; action: ActionDto }
	/** A lookup missed or was ambiguous; the panel offers these to pick from. */
	| { type: 'choices'; options: string[] }
	| { type: 'end'; stopped_early: boolean }
	/** A failure after the stream opened; there is no status code left by then. */
	| { type: 'error'; message: string };

export type MessageDto = {
	role: 'user' | 'assistant';
	content: string;
	tool_name: string | null;
	created_at: number;
};

export type ConversationSummaryDto = {
	id: string;
	title: string;
	updated_at: number;
	/** Changes it proposed that nobody has settled yet. */
	pending_actions: number;
	/** Only set when the row came from a search. `"title" | "content" | "both"`. */
	match_in?: 'title' | 'content' | 'both';
	/** Excerpt around the first content hit, only when `match_in` is `"content"` or `"both"`. */
	snippet?: string;
};

export type ConversationDto = {
	id: string;
	title: string;
	messages: MessageDto[];
	actions: ActionDto[];
};

const base = (organizationId: string) =>
	`/api/v1/organizations/${encodeURIComponent(organizationId)}/assistant`;

/**
 * Sends one message and calls `onEvent` for each thing the assistant does.
 *
 * Server-sent events over `POST`, so this is `fetch` and a reader rather than
 * `EventSource`, which is GET-only. Resolves when the turn ends.
 */
export async function streamMessage(
	organizationId: string,
	message: string,
	conversationId: string | null,
	onEvent: (event: TurnEvent) => void,
	signal?: AbortSignal,
	/** Only read when starting a conversation; an existing one keeps what it
	 *  was pinned to, so switching model mid-thread is deliberately a no-op. */
	model?: { connection_id: string; model: string } | null,
): Promise<void> {
	const response = await fetch(`${API_URL}${base(organizationId)}/messages`, {
		method: 'POST',
		credentials: 'include',
		headers: { 'Content-Type': 'application/json' },
		body: JSON.stringify({ message, conversation_id: conversationId, ...(model ?? {}) }),
		signal,
	});

	if (!response.ok || !response.body) {
		// A failure before the stream opened still has a status and a JSON body.
		const raw = await response.text();
		let detail = `${response.status} ${response.statusText}`;
		try {
			detail = JSON.parse(raw).error ?? detail;
		} catch {
			// A non-JSON body (a proxy error page) leaves the status line as the message.
		}
		throw new Error(detail);
	}

	const reader = response.body.getReader();
	const decoder = new TextDecoder();
	let buffer = '';

	for (;;) {
		const { done, value } = await reader.read();
		if (done) break;
		buffer += decoder.decode(value, { stream: true });

		// SSE frames are separated by a blank line; a partial one stays buffered.
		let split = buffer.indexOf('\n\n');
		while (split !== -1) {
			const frame = buffer.slice(0, split);
			buffer = buffer.slice(split + 2);
			const data = frame
				.split('\n')
				.filter((line) => line.startsWith('data:'))
				.map((line) => line.slice(5).trim())
				.join('');
			if (data) {
				try {
					onEvent(JSON.parse(data) as TurnEvent);
				} catch {
					// A frame we cannot parse is a bug upstream, not a reason to drop
					// the rest of the turn.
				}
			}
			split = buffer.indexOf('\n\n');
		}
	}
}

export async function listConversations(
	organizationId: string,
	query?: string,
): Promise<ConversationSummaryDto[]> {
	const trimmed = query?.trim();
	const url = trimmed
		? `${base(organizationId)}/conversations?q=${encodeURIComponent(trimmed)}`
		: `${base(organizationId)}/conversations`;
	return (await api<ConversationSummaryDto[]>(url)).data!;
}

export async function readConversation(
	organizationId: string,
	conversationId: string,
): Promise<ConversationDto> {
	return (
		await api<ConversationDto>(
			`${base(organizationId)}/conversations/${encodeURIComponent(conversationId)}`,
		)
	).data!;
}

export async function renameConversation(
	organizationId: string,
	conversationId: string,
	title: string,
): Promise<ConversationSummaryDto> {
	return (
		await api<ConversationSummaryDto>(
			`${base(organizationId)}/conversations/${encodeURIComponent(conversationId)}`,
			{ method: 'PATCH', body: JSON.stringify({ title }) },
		)
	).data!;
}

export async function settleAction(
	organizationId: string,
	actionId: string,
	decision: 'confirm' | 'reject',
): Promise<ActionDto> {
	return (
		await api<ActionDto>(`${base(organizationId)}/actions/${encodeURIComponent(actionId)}`, {
			method: 'POST',
			body: JSON.stringify({ decision }),
		})
	).data!;
}
