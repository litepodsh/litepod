import { setMode, userPrefersMode } from 'mode-watcher';

type Mode = 'light' | 'dark' | 'system';

/**
 * Switch theme with a circular wipe expanding from the click point.
 *
 * The wipe is a WAAPI animation on `::view-transition-new(root)`, which the
 * blanket `prefers-reduced-motion` rule in `app.css` cannot reach — that rule
 * only overrides CSS `animation-duration`, and WAAPI ignores it. So the guard
 * has to live here, or a user who asked for reduced motion still gets a
 * 600ms full-screen sweep.
 */
export function switchThemeMode(
	value: Mode,
	origin: { x: number; y: number },
	/** Extra DOM work to capture in the same transition, e.g. closing a menu. */
	alsoApply?: () => void,
): void {
	if (value === userPrefersMode.current) {
		alsoApply?.();
		return;
	}

	const apply = () => {
		setMode(value);
		alsoApply?.();
	};

	const reduced =
		typeof window !== 'undefined' && window.matchMedia('(prefers-reduced-motion: reduce)').matches;

	if (reduced || !document.startViewTransition) {
		apply();
		return;
	}

	const { x, y } = origin;
	const endRadius = Math.hypot(
		Math.max(x, window.innerWidth - x),
		Math.max(y, window.innerHeight - y),
	);
	const transition = document.startViewTransition(apply);
	transition.ready
		.then(() => {
			document.documentElement.animate(
				{ clipPath: [`circle(0px at ${x}px ${y}px)`, `circle(${endRadius}px at ${x}px ${y}px)`] },
				{
					duration: 600,
					easing: 'ease-in-out',
					pseudoElement: '::view-transition-new(root)',
				},
			);
		})
		.catch(() => {});
}
