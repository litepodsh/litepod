/**
 * Where the assistant lives and how wide its column is.
 *
 * The assistant shares the sidebar's column rather than adding a second rail:
 * `nav` and `chat` are the two things that one column can show, and `detached`
 * pulls the assistant out to the opposite side while the sidebar drops to its
 * icon rail. Navigation and conversation are never used in the same second, so
 * they trade one column instead of each claiming 16rem of chrome.
 *
 * Widths override `--sidebar-width` / `--assistant-width` on the sidebar
 * wrapper, so resizing is one custom property and nothing re-lays out.
 */
export type AssistantMode = 'nav' | 'chat' | 'detached';
export type AssistantView = 'thread' | 'history' | 'empty';

const MODE_KEY = 'litepod-assistant-mode';
const SIDEBAR_WIDTH_KEY = 'litepod-sidebar-width';
const ASSISTANT_WIDTH_KEY = 'litepod-assistant-width';

/** Rem. The minimum is `SIDEBAR_WIDTH`, so the two rails start out mirrored. */
export const SIDEBAR_WIDTH_MIN = 16;
export const SIDEBAR_WIDTH_MAX = 34;
export const ASSISTANT_WIDTH_MIN = 16;
export const ASSISTANT_WIDTH_MAX = 40;

export const assistantState = $state<{
	mode: AssistantMode;
	view: AssistantView;
	unread: boolean;
	/**
	 * The selected organization has the assistant turned on. Set once, by the
	 * sidebar, which is the only place that loads organizations; everything else
	 * reads it so there is one answer rather than one request per surface.
	 *
	 * Presentation only: the API refuses these routes on its own.
	 */
	available: boolean;
	sidebarWidth: number;
	assistantWidth: number;
}>({
	mode: 'nav',
	view: 'thread',
	unread: false,
	available: false,
	sidebarWidth: SIDEBAR_WIDTH_MIN,
	assistantWidth: 22,
});

function clamp(value: number, min: number, max: number) {
	return Math.min(max, Math.max(min, value));
}

function readWidth(key: string, fallback: number, min: number, max: number) {
	const stored = Number(window.localStorage.getItem(key));
	return Number.isFinite(stored) && stored > 0 ? clamp(stored, min, max) : fallback;
}

export function restoreAssistant() {
	if (typeof window === 'undefined') return;
	const mode = window.localStorage.getItem(MODE_KEY);
	if (mode === 'chat' || mode === 'detached') assistantState.mode = mode;
	assistantState.sidebarWidth = readWidth(
		SIDEBAR_WIDTH_KEY,
		SIDEBAR_WIDTH_MIN,
		SIDEBAR_WIDTH_MIN,
		SIDEBAR_WIDTH_MAX,
	);
	assistantState.assistantWidth = readWidth(
		ASSISTANT_WIDTH_KEY,
		22,
		ASSISTANT_WIDTH_MIN,
		ASSISTANT_WIDTH_MAX,
	);
}

export function setAssistantMode(mode: AssistantMode) {
	assistantState.mode = mode;
	if (mode !== 'nav') assistantState.unread = false;
	if (typeof window !== 'undefined') window.localStorage.setItem(MODE_KEY, mode);
}

export function setSidebarWidth(rem: number) {
	assistantState.sidebarWidth = clamp(rem, SIDEBAR_WIDTH_MIN, SIDEBAR_WIDTH_MAX);
	if (typeof window !== 'undefined')
		window.localStorage.setItem(SIDEBAR_WIDTH_KEY, String(assistantState.sidebarWidth));
}

export function setAssistantWidth(rem: number) {
	assistantState.assistantWidth = clamp(rem, ASSISTANT_WIDTH_MIN, ASSISTANT_WIDTH_MAX);
	if (typeof window !== 'undefined')
		window.localStorage.setItem(ASSISTANT_WIDTH_KEY, String(assistantState.assistantWidth));
}
