/** Shared helpers for the Cloudflare Turnstile widget, used on the register (always-on,
 *  cloud only) and login (adaptive, after repeated failures) pages. */

/** Injects the Turnstile script once per page load. Safe to call more than once. */
export function loadTurnstileScript() {
	if (document.querySelector('script[data-turnstile]')) return;
	const script = document.createElement('script');
	script.src = 'https://challenges.cloudflare.com/turnstile/v0/api.js';
	script.async = true;
	script.defer = true;
	script.dataset.turnstile = 'true';
	document.head.appendChild(script);
}

/** Reads the token from the hidden `cf-turnstile-response` input the widget injects into
 *  its container div once solved. Empty string when unsolved. */
export function readTurnstileToken(widgetEl: HTMLElement | undefined): string {
	return (
		widgetEl?.querySelector<HTMLInputElement>('input[name="cf-turnstile-response"]')?.value ?? ''
	);
}

/** Tokens are single-use: reset before letting the user retry after a rejected submit,
 *  otherwise a naive retry replays the redeemed token and Cloudflare 400s it. */
export function resetTurnstile(widgetEl: HTMLElement | undefined) {
	window.turnstile?.reset(widgetEl);
}
