import { toast as base } from 'svelte-sonner';
import type { ExternalToast } from 'svelte-sonner';
import { copyText } from './clipboard.js';

function copyAction(message: string) {
	return {
		label: 'Copy',
		onClick: () => {
			void copyText(message);
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
