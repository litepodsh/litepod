import { browser } from '$app/env';

const CHANNEL_NAME = 'litepod-auth';
const LOGOUT_EVENT = 'logout';

type AuthEvent = { type: typeof LOGOUT_EVENT };

let channel: BroadcastChannel | null = null;

function getChannel(): BroadcastChannel | null {
	if (!browser) return null;
	if (channel) return channel;
	if (typeof BroadcastChannel === 'undefined') return null;
	channel = new BroadcastChannel(CHANNEL_NAME);
	return channel;
}

/** Notify other open tabs that this user just logged out. */
export function broadcastLogout(): void {
	const ch = getChannel();
	ch?.postMessage({ type: LOGOUT_EVENT } satisfies AuthEvent);
}

/** Subscribe to cross-tab logout notifications. Returns an unsubscribe fn. */
export function onAuthMessage(handler: (event: AuthEvent) => void): () => void {
	const ch = getChannel();
	if (!ch) return () => {};
	const listener = (event: MessageEvent<AuthEvent>) => {
		if (event.data?.type === LOGOUT_EVENT) {
			handler(event.data);
		}
	};
	ch.addEventListener('message', listener);
	return () => ch.removeEventListener('message', listener);
}
