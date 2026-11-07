/**
 * Runs `fn` every `ms` while the tab is visible. Hidden tabs skip their ticks and
 * refresh once on the way back, so a backgrounded panel costs no requests.
 * Returns the cleanup. Does not run `fn` immediately: callers do their own first load.
 */
export function pollWhileVisible(fn: () => unknown, ms: number): () => void {
	const timer = setInterval(() => {
		if (!document.hidden) void fn();
	}, ms);
	const onVisibilityChange = () => {
		if (!document.hidden) void fn();
	};
	document.addEventListener('visibilitychange', onVisibilityChange);
	return () => {
		clearInterval(timer);
		document.removeEventListener('visibilitychange', onVisibilityChange);
	};
}
