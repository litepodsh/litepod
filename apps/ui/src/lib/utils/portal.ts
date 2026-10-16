import type { Action } from 'svelte/action';

/** Mount the node under a different ancestor (default `document.body`) so it escapes
 *  any clipping/transform on the original parent. Used for hover popovers that need
 *  to paint over the card's own border-radius without being trapped inside it. */
export const portal: Action<HTMLElement, string | HTMLElement> = (node, target) => {
	let current: HTMLElement | null = null;
	function move(to: HTMLElement | string) {
		const dest = typeof to === 'string' ? document.querySelector(to) : to;
		if (!(dest instanceof HTMLElement) || dest === current) return;
		dest.appendChild(node);
		current = dest;
	}
	if (target) move(target);
	return {
		update(next) {
			move(next);
		},
		destroy() {
			node.remove();
		},
	};
};
