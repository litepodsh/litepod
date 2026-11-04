const PRECISE_POINTER_QUERY = '(hover: hover) and (pointer: fine)';

/**
 * True when the primary input is a mouse or trackpad. Phones and tablets report
 * a coarse pointer, so they are excluded even if the viewport is wide.
 */
export function hasPrecisePointer(): boolean {
	if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return false;
	return window.matchMedia(PRECISE_POINTER_QUERY).matches;
}

/** True when the user is typing somewhere else or has a dialog open. */
function isUserBusy(active: Element | null): boolean {
	if (!active) return false;
	if (active instanceof HTMLElement) {
		if (active.isContentEditable) return true;
		if (active.closest('input, textarea, select, [role="dialog"], [aria-modal="true"]'))
			return true;
	}
	return false;
}

/**
 * Focus a search field on mount, but only on precise-pointer devices. Touch
 * devices skip it so the on-screen keyboard does not open on page load.
 * Returns whether focus moved.
 */
export function focusOnDesktop(element: HTMLElement | null | undefined): boolean {
	if (!element || !hasPrecisePointer()) return false;
	if (isUserBusy(document.activeElement)) return false;
	element.focus({ preventScroll: true });
	return true;
}
