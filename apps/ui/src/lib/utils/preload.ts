import { preloadCode, preloadData } from '$app/navigation';
import type { Action } from 'svelte/action';

const warmed = new Set<string>();

/**
 * Preload a route's code and data once, so the click that follows navigates
 * instantly. Deduped: a route already in flight is not requested twice.
 */
export function prefetchRoute(href: string): void {
	if (!href || warmed.has(href)) return;
	warmed.add(href);
	// Typed routes narrow these helpers to the known route union; sidebar links
	// carry dynamic ids, so widen back to a plain runtime path.
	void Promise.allSettled([preloadCode(href as never), preloadData(href as never)]).finally(() => {
		warmed.delete(href);
	});
}

/**
 * Warm a link's route as soon as the pointer rests on it (and on keyboard focus),
 * mirroring SvelteKit's `data-sveltekit-preload-data="hover"`. Debounced so
 * sweeping across the sidebar does not fetch every link it passes over.
 */
export const preloadOnHover: Action<HTMLAnchorElement, string> = (node, href) => {
	let current = href;
	let timer: ReturnType<typeof setTimeout> | undefined;

	function schedule() {
		clearTimeout(timer);
		timer = setTimeout(() => prefetchRoute(current), 80);
	}

	function cancel() {
		clearTimeout(timer);
	}

	function focus() {
		prefetchRoute(current);
	}

	node.addEventListener('pointerenter', schedule);
	node.addEventListener('pointerleave', cancel);
	node.addEventListener('focus', focus);

	return {
		update(next: string) {
			current = next;
		},
		destroy() {
			cancel();
			node.removeEventListener('pointerenter', schedule);
			node.removeEventListener('pointerleave', cancel);
			node.removeEventListener('focus', focus);
		},
	};
};
