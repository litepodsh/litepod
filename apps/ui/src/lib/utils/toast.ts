import { toast as base } from 'svelte-sonner';
import type { ExternalToast } from 'svelte-sonner';

function copyAction(message: string) {
	return {
		label: 'Copy',
		onClick: () => {
			navigator.clipboard.writeText(message).catch(() => {
				// fallback for older browsers / non-secure contexts
				const ta = document.createElement('textarea');
				ta.value = message;
				ta.style.position = 'fixed';
				ta.style.opacity = '0';
				document.body.appendChild(ta);
				ta.select();
				document.execCommand('copy');
				document.body.removeChild(ta);
			});
		},
	};
}

/** Wrapped toast with copy button on every .error() call. */
export const toast = {
	...base,
	error: (message: string | Error, data?: ExternalToast) => {
		const msg = message instanceof Error ? message.message : message;
		return base.error(msg, {
			...data,
			action: data?.action ?? copyAction(msg),
		});
	},
} as const;

/** Reports a caught error: its own message if it's an `Error`, `fallback` otherwise.
 *  Saves the `err instanceof Error ? err.message : '...'` ternary repeated at every
 *  catch site across the app. */
export function toastError(err: unknown, fallback: string) {
	toast.error(err instanceof Error ? err : fallback);
}
