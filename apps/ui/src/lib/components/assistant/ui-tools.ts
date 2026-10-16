/**
 * Taking the user to the page a change lands on.
 *
 * The API resolves the route when it stores the action, so the panel does not
 * guess at one. Confirming navigates first, so the user watches the change
 * happen on the real page instead of being told it did.
 */
import { goto } from '$app/navigation';
import { tick } from 'svelte';

/** Route prefixes the assistant may send the user to. Anything else is refused. */
const ALLOWED_PREFIXES = [
	'/home',
	'/projects',
	'/organizations',
	'/notifications',
	'/git-providers',
	'/settings',
	'/system',
];

export function isSafeRoute(route: string): boolean {
	// Reject protocol-relative and absolute URLs before looking at the prefix,
	// so `//evil.example` and `https://…` never reach `goto`.
	if (!route.startsWith('/') || route.startsWith('//')) return false;
	return ALLOWED_PREFIXES.some(
		(prefix) =>
			route === prefix || route.startsWith(`${prefix}/`) || route.startsWith(`${prefix}?`),
	);
}

/** The tail the assistant is given. Enough to see a failure, short enough that
 *  one noisy build does not become the whole conversation. */
export const LOG_LINES_FOR_ASSISTANT = 200;

/** The last `LOG_LINES_FOR_ASSISTANT` lines, oldest first. */
export function tailForAssistant(text: string): string {
	return text.split('\n').slice(-LOG_LINES_FOR_ASSISTANT).join('\n');
}

/**
 * The question the log viewers send.
 *
 * It names the application and pastes the tail; where the application came from
 * (an internal template, a catalog template, or the user) is left for the
 * assistant to read with `get`, so the answer comes from the system rather than
 * from whatever the page happened to have in scope.
 */
export function logQuestion(
	project: string,
	application: string,
	kind: 'deployment' | 'runtime',
	tail: string,
) {
	const source = kind === 'deployment' ? 'deployment' : 'runtime';
	return [
		`Context: project \`${project}\`, application \`${application}\`.`,
		`Here are the last ${LOG_LINES_FOR_ASSISTANT} ${source} log lines.`,
		'Check what kind of application it is and where it came from, tell me what is wrong,',
		'and propose the change that fixes it. If that is a compose change, show me what you',
		'would change and ask before writing it.',
		'',
		'```',
		tail,
		'```',
	].join('\n');
}

/** Navigates, and resolves only once the route has settled. */
export async function navigateToAction(route: string): Promise<boolean> {
	if (!isSafeRoute(route)) return false;
	try {
		await goto(route);
	} catch {
		return false; // kit 3: goto rejects routes that do not resolve
	}
	await tick();
	return true;
}
