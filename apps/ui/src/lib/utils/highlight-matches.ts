export type HighlightSegment = { text: string; highlighted: boolean };

export function highlightMatches(
	text: string,
	indices: readonly (readonly [number, number])[] | undefined,
): HighlightSegment[] {
	if (!indices || indices.length === 0) return [{ text, highlighted: false }];

	const sorted = [...indices].sort((a, b) => a[0] - b[0]);
	const merged: [number, number][] = [];
	for (const [start, end] of sorted) {
		const last = merged[merged.length - 1];
		if (last && start <= last[1] + 1) {
			last[1] = Math.max(last[1], end);
		} else {
			merged.push([start, end]);
		}
	}

	const segments: HighlightSegment[] = [];
	let cursor = 0;
	for (const [start, end] of merged) {
		if (start > cursor) segments.push({ text: text.slice(cursor, start), highlighted: false });
		segments.push({ text: text.slice(start, end + 1), highlighted: true });
		cursor = end + 1;
	}
	if (cursor < text.length) segments.push({ text: text.slice(cursor), highlighted: false });
	return segments;
}
