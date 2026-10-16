import { i18n, t } from '#lib/i18n/index.svelte.js';

// Five-field cron (minute hour day-of-month month day-of-week), evaluated in UTC
// like the API scheduler. Supports `*`, lists, ranges and steps.
export type CronSpec = { sets: Set<number>[]; domAny: boolean; dowAny: boolean };

const LIMITS: [number, number][] = [
	[0, 59],
	[0, 23],
	[1, 31],
	[1, 12],
	[0, 7],
];

function field(f: string, min: number, max: number): Set<number> | null {
	const out = new Set<number>();
	for (const part of f.split(',')) {
		const [range, stepRaw] = part.split('/');
		const step = stepRaw === undefined ? 1 : Number(stepRaw);
		let a = min;
		let b = max;
		if (range !== '*') {
			if (range.includes('-')) [a, b] = range.split('-').map(Number);
			else {
				a = Number(range);
				b = stepRaw === undefined ? a : max;
			}
		}
		if ([a, b, step].some((n) => !Number.isInteger(n)) || a < min || b > max || a > b || step < 1)
			return null;
		for (let v = a; v <= b; v += step) out.add(v);
	}
	return out;
}

export function parseCron(expr: string): CronSpec | null {
	const f = expr.trim().split(/\s+/);
	if (f.length !== 5) return null;
	const sets = f.map((x, i) => field(x, ...LIMITS[i]));
	if (sets.some((s) => !s)) return null;
	if (sets[4]!.has(7)) sets[4]!.add(0);
	return { sets: sets as Set<number>[], domAny: f[2] === '*', dowAny: f[4] === '*' };
}

// ponytail: minute-by-minute scan, capped at ~400 days; fine for a preview of 3 runs.
export function nextRuns(expr: string, count = 3, from = new Date()): Date[] {
	const spec = parseCron(expr);
	if (!spec) return [];
	const [M, H, DM, MO, DW] = spec.sets;
	const d = new Date(from);
	d.setUTCSeconds(0, 0);
	d.setUTCMinutes(d.getUTCMinutes() + 1);
	const out: Date[] = [];
	for (let i = 0; i < 60 * 24 * 400 && out.length < count; i++) {
		const dm = DM.has(d.getUTCDate());
		const dw = DW.has(d.getUTCDay());
		// Standard cron: when both day fields are restricted, either one matching is enough.
		const day = spec.domAny && spec.dowAny ? true : spec.domAny ? dw : spec.dowAny ? dm : dm || dw;
		if (M.has(d.getUTCMinutes()) && H.has(d.getUTCHours()) && MO.has(d.getUTCMonth() + 1) && day)
			out.push(new Date(d));
		d.setUTCMinutes(d.getUTCMinutes() + 1);
	}
	return out;
}

export function describeCron(expr: string): string | null {
	if (!parseCron(expr)) return null;
	const [m, h, dom, mon, dow] = expr.trim().split(/\s+/);
	const pad = (x: string) => x.padStart(2, '0');
	const num = /^\d+$/;
	const every = /^\*\/\d+$/;
	const time = `${pad(h)}:${pad(m)}`;
	if (every.test(m) && h === '*' && dom === '*' && mon === '*' && dow === '*')
		return t('common.cron.every_minutes', { count: m.slice(2) });
	if (num.test(m) && h === '*' && dom === '*' && dow === '*')
		return t('common.cron.hourly', { minute: pad(m) });
	if (num.test(m) && every.test(h) && dom === '*' && dow === '*')
		return t('common.cron.every_hours', { count: h.slice(2), minute: pad(m) });
	if (num.test(m) && num.test(h) && dom === '*' && mon === '*' && dow === '*')
		return t('common.cron.every_day', { time });
	if (num.test(m) && num.test(h) && dom === '*' && /^\d$/.test(dow)) {
		// 2023-01-01 was a Sunday, so day N of that week is weekday N
		const day = new Date(Date.UTC(2023, 0, 1 + (Number(dow) % 7))).toLocaleDateString(i18n.locale, {
			weekday: 'long',
			timeZone: 'UTC',
		});
		return t('common.cron.every_weekday', { day, time });
	}
	if (num.test(m) && num.test(h) && num.test(dom))
		return t('common.cron.monthly', { day: dom, time });
	return t('common.cron.custom');
}
