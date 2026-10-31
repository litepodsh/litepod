/**
 * The transcript the panel renders, and the one place a turn is run.
 *
 * The API streams a turn as it happens; this folds those events into a flat
 * list of items so the panel is a `{#each}` and nothing else.
 */
import {
	listConversations,
	readConversation,
	settleAction,
	streamMessage,
	type ActionDto,
	type ConversationSummaryDto,
	type TurnEvent,
} from '#lib/clients/assistant-client.js';
import { logQuestion, navigateToAction, tailForAssistant } from './ui-tools';
import { assistantState, setAssistantMode } from '#lib/state/assistant.svelte.js';

export type ThreadItem =
	| { kind: 'user'; text: string }
	| { kind: 'assistant'; text: string }
	| {
			kind: 'step';
			id: string;
			name: string;
			arguments: string;
			running: boolean;
			ok: boolean;
			/** `Date.now()` at start, and the elapsed time once finished. Steps
			 *  restored from storage have neither. */
			startedAt?: number;
			durationMs?: number;
	  }
	| { kind: 'action'; action: ActionDto }
	| { kind: 'choices'; options: string[] }
	| { kind: 'stopped'; text: string };

/** So the panel comes back to the same conversation after a reload. */
const CONVERSATION_KEY = 'litepod-assistant-conversation';
/** And to the same model. Stored as `<connection id>:<model id>`. */
const MODEL_KEY = 'litepod-assistant-model';

export const thread = $state<{
	items: ThreadItem[];
	conversationId: string | null;
	running: boolean;
	error: string | null;
	pendingDraft: string | null;
	pendingDraftNotice: boolean;
	/** The picker's selection, `<connection id>:<model id>`. Null means "use
	 *  the organization's default", which is what a fresh install has. */
	model: string | null;
}>({
	items: [],
	conversationId: null,
	running: false,
	error: null,
	pendingDraft: null,
	pendingDraftNotice: false,
	model: readModel(),
});

function readModel(): string | null {
	try {
		return window.localStorage.getItem(MODEL_KEY);
	} catch {
		return null;
	}
}

/**
 * Picks the model for the next conversation.
 *
 * It does not change the open one: a thread stays on what it started with, so
 * its transcript and its model never disagree. The API enforces the same rule.
 */
export function chooseModel(selection: string | null) {
	thread.model = selection;
	try {
		if (selection) window.localStorage.setItem(MODEL_KEY, selection);
		else window.localStorage.removeItem(MODEL_KEY);
	} catch {
		// Storage blocked: the choice still applies for this session.
	}
}

/** Splits the picker's `<connection id>:<model id>` into what the API wants. */
function chosenModel(): { connection_id: string; model: string } | null {
	if (!thread.model) return null;
	const separator = thread.model.indexOf(':');
	if (separator === -1) return null;
	return {
		connection_id: thread.model.slice(0, separator),
		model: thread.model.slice(separator + 1),
	};
}

/** Lets the panel stop a turn, and stops it on unmount. */
let inFlight: AbortController | null = null;

export function resetThread() {
	cancelTurn();
	thread.items = [];
	rememberConversation(null);
	thread.error = null;
}

function rememberConversation(id: string | null) {
	thread.conversationId = id;
	try {
		if (id) window.localStorage.setItem(CONVERSATION_KEY, id);
		else window.localStorage.removeItem(CONVERSATION_KEY);
	} catch {
		// A private window with storage blocked still gets a working panel; it
		// just forgets the conversation on reload.
	}
}

/** The conversations this user has in this organization, newest first. */
export async function conversations(organizationId: string): Promise<ConversationSummaryDto[]> {
	return listConversations(organizationId);
}

/**
 * Reopens whatever was on screen last.
 *
 * A conversation belongs to one organization, so one that does not load - the
 * user switched organizations, or it was deleted - is dropped rather than
 * reported: there is nothing the reader could do about it.
 */
export async function restoreLast(organizationId: string) {
	if (thread.conversationId || thread.items.length) return;
	let stored: string | null = null;
	try {
		stored = window.localStorage.getItem(CONVERSATION_KEY);
	} catch {
		return;
	}
	if (!stored) return;
	try {
		await restore(organizationId, stored);
	} catch {
		rememberConversation(null);
	}
}

export function cancelTurn() {
	inFlight?.abort();
	inFlight = null;
	thread.running = false;
}

function applyEvent(event: TurnEvent) {
	switch (event.type) {
		case 'conversation':
			rememberConversation(event.id);
			return;
		case 'text':
			// Rig sends token deltas. Keep a model turn in one bubble; a tool
			// between turns naturally starts the next bubble.
			if (thread.items.at(-1)?.kind === 'assistant') {
				(thread.items.at(-1) as Extract<ThreadItem, { kind: 'assistant' }>).text += event.text;
			} else {
				thread.items.push({ kind: 'assistant', text: event.text });
			}
			return;
		case 'tool_started':
			thread.items.push({
				kind: 'step',
				id: event.id,
				name: event.name,
				arguments: event.arguments,
				running: true,
				ok: true,
			});
			return;
		case 'tool_finished': {
			const step = thread.items.find((item) => item.kind === 'step' && item.id === event.id);
			if (step?.kind !== 'step') return;
			step.running = false;
			step.ok = event.ok;
			return;
		}
		case 'action':
			thread.items.push({ kind: 'action', action: event.action });
			return;
		case 'choices':
			thread.items.push({ kind: 'choices', options: event.options });
			return;
		case 'error':
			thread.error = event.message;
			return;
		case 'end':
			// Worth saying: the answer above is a stop, not a conclusion.
			if (event.stopped_early) {
				thread.items.push({
					kind: 'stopped',
					text: 'The assistant ran out of steps before finishing. Narrow the question and ask again.',
				});
			}
			return;
	}
}

export async function ask(organizationId: string, question: string) {
	const message = question.trim();
	if (!message || thread.running) return;

	thread.items.push({ kind: 'user', text: message });
	thread.running = true;
	thread.error = null;

	const controller = new AbortController();
	inFlight = controller;
	try {
		await streamMessage(
			organizationId,
			message,
			thread.conversationId,
			applyEvent,
			controller.signal,
			chosenModel(),
		);
	} catch (error) {
		// An aborted turn is the user's own doing, not a failure to report.
		if (!controller.signal.aborted) {
			thread.error = error instanceof Error ? error.message : 'The assistant did not answer.';
		}
	} finally {
		if (inFlight === controller) inFlight = null;
		thread.running = false;
	}
}

/**
 * What the "Ask AI" button in a log viewer does: open the assistant and hand it
 * the tail, so the user does not have to copy, paste and explain.
 */
export function askAboutLogs(
	project: string,
	application: string,
	kind: 'deployment' | 'runtime',
	logs: string,
) {
	// Stay where the user already has the assistant: dock it into the sidebar
	// only if it was closed. Yanking a docked assistant out to a floating rail
	// (or vice versa) every time they click "Ask AI" makes the page jiggle.
	if (assistantState.mode === 'nav') {
		setAssistantMode('chat');
	}
	assistantState.view = 'thread';
	thread.pendingDraft = logQuestion(project, application, kind, tailForAssistant(logs));
	thread.pendingDraftNotice = true;
}

/**
 * Confirming is the only thing that writes. The panel navigates to the page the
 * change lands on first, so the user watches it happen rather than being told
 * it did.
 */
export async function settle(
	organizationId: string,
	action: ActionDto,
	decision: 'confirm' | 'reject',
) {
	const card = thread.items.find((item) => item.kind === 'action' && item.action.id === action.id);

	// Best effort, and deliberately before the write so the page is already open
	// when the row lands. A route that no longer exists must never be the reason
	// a confirmed change does not happen, so its failure is swallowed here.
	if (decision === 'confirm') {
		try {
			await navigateToAction(action.route);
		} catch {
			// Stay where we are; the change still goes through below.
		}
	}

	try {
		const settled = await settleAction(organizationId, action.id, decision);
		if (card?.kind === 'action') card.action = settled;
	} catch (error) {
		thread.error = error instanceof Error ? error.message : 'That change could not be applied.';
		await syncAction(organizationId, action.id, card);
	}
}

/**
 * A confirm that errors can still have settled the action: the API marks it
 * `failed` and keeps the reason. Without re-reading it, the card stays
 * `pending` and a second click only gets "already failed".
 */
async function syncAction(
	organizationId: string,
	actionId: string,
	card: ThreadItem | undefined,
) {
	if (card?.kind !== 'action' || !thread.conversationId) return;
	try {
		const conversation = await readConversation(organizationId, thread.conversationId);
		const fresh = conversation.actions.find((candidate) => candidate.id === actionId);
		if (fresh) card.action = fresh;
	} catch {
		// The error above is already on screen; the card just stays as it was.
	}
}

/** Reopens a stored conversation, so the panel survives a reload. */
export async function restore(organizationId: string, conversationId: string) {
	const conversation = await readConversation(organizationId, conversationId);
	rememberConversation(conversation.id);
	thread.items = [
		...conversation.messages.map((message) =>
			message.role === 'user'
				? ({ kind: 'user', text: message.content } as const)
				: ({ kind: 'assistant', text: message.content } as const),
		),
		...conversation.actions.map((action) => ({ kind: 'action', action }) as const),
	];
}
