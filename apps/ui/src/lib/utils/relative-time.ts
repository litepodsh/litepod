const STEPS: [Intl.RelativeTimeFormatUnit, number][] = [
	['second', 60],
	['minute', 60],
	['hour', 24],
	['day', 30],
	['month', 12],
	['year', Infinity],
];

/** "in 5 minutes" / "hace 2 horas", in the UI locale. Timestamps are milliseconds. */
export function relativeTime(timestamp: number, now = Date.now(), locale = 'en'): string {
	let value = (timestamp - now) / 1000;
	for (const [unit, size] of STEPS) {
		if (Math.abs(value) < size)
			return new Intl.RelativeTimeFormat(locale, { numeric: 'auto' }).format(
				Math.round(value),
				unit,
			);
		value /= size;
	}
	return '';
}
