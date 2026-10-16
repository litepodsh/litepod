import type { DeliveryStatus } from '#lib/clients/notification-client.js';
import { toneBorder, toneDot as sharedDot, type StatusTone } from '#lib/utils/status-tone.js';

const MAX_ATTEMPTS = 5;

/** Statuses that mean the queue is still working on this row. */
export function isInFlight(status: DeliveryStatus): boolean {
	return status === 'pending' || status === 'sending';
}

export function statusLabel(status: DeliveryStatus, attempts: number): string {
	switch (status) {
		case 'delivered':
			return 'Delivered';
		case 'sending':
			return 'Sending…';
		case 'pending':
			// A pending row with attempts on it is a retry, not a first try.
			return attempts > 0 ? `Retrying (${attempts}/${MAX_ATTEMPTS})` : 'Queued';
		case 'dead':
		case 'failed':
			return 'Failed';
	}
}

function deliveryTone(status: DeliveryStatus): StatusTone {
	switch (status) {
		case 'delivered':
			return 'ok';
		case 'sending':
		case 'pending':
			return 'warn';
		case 'dead':
		case 'failed':
			return 'danger';
	}
}

export function statusTone(status: DeliveryStatus): string {
	return toneBorder(deliveryTone(status));
}

export function statusDot(status: DeliveryStatus): string {
	return sharedDot(deliveryTone(status));
}

export function relativeTime(timestamp: number): string {
	const seconds = Math.round((Date.now() - timestamp) / 1000);
	if (seconds < 60) return 'just now';
	const minutes = Math.round(seconds / 60);
	if (minutes < 60) return `${minutes}m ago`;
	const hours = Math.round(minutes / 60);
	if (hours < 24) return `${hours}h ago`;
	return `${Math.round(hours / 24)}d ago`;
}
