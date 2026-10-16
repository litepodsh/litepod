export function formatLogTimestamp(ts: number | string | null | undefined): string {
	if (ts == null) return '';
	const d = typeof ts === 'string' ? new Date(ts) : new Date(ts);
	const hh = d.getHours().toString().padStart(2, '0');
	const mm = d.getMinutes().toString().padStart(2, '0');
	const ss = d.getSeconds().toString().padStart(2, '0');
	const mmm = d.getMilliseconds().toString().padStart(3, '0');
	return `${hh}:${mm}:${ss}.${mmm}`;
}
