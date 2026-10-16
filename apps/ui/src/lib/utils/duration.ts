const UNITS: [label: string, seconds: number][] = [
	['d', 86_400],
	['h', 3_600],
	['m', 60],
	['s', 1],
];

/** "90061" -> "1d 1h 1m 1s", keeping at most the two largest non-zero units. */
export function formatDuration(totalSeconds: number): string {
	const seconds = Math.max(0, Math.floor(totalSeconds));
	if (seconds === 0) return '0s';

	const parts: string[] = [];
	let rest = seconds;
	for (const [label, size] of UNITS) {
		const value = Math.floor(rest / size);
		rest -= value * size;
		if (value > 0) parts.push(`${value}${label}`);
		if (parts.length === 2) break;
	}
	return parts.join(' ');
}

/** Live relative label: "never", "12s ago", "3m 20s ago". */
export function formatRelative(iso: string | null, now: number = Date.now()): string {
	if (!iso) return 'never';
	const then = new Date(iso).getTime();
	if (Number.isNaN(then)) return 'never';
	const seconds = Math.floor((now - then) / 1_000);
	if (seconds < 1) return 'just now';
	return `${formatDuration(seconds)} ago`;
}
