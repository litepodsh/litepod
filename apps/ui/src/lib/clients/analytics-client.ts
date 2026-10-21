import { api } from './base-client';

/** HTTP request analytics from Caddy access logs (DuckDB on the API). */
export type AnalyticsRange = '1h' | '24h' | '7d' | '30d';
export const ANALYTICS_RANGES: AnalyticsRange[] = ['1h', '24h', '7d', '30d'];

export type AnalyticsPoint = {
	/** Bucket start, epoch milliseconds. */
	ts: number;
	requests: number;
	errors_4xx: number;
	errors_5xx: number;
	bytes_out: number;
	avg_ms: number | null;
	p95_ms: number | null;
	/** Distinct non-bot visitors in this bucket. */
	visitors: number;
	/** Requests per application in this bucket, busiest first (organization view only). */
	applications?: { key: string; requests: number }[];
};

export type AnalyticsTotals = {
	requests: number;
	errors_4xx: number;
	errors_5xx: number;
	bytes_out: number;
	bytes_in: number;
	avg_ms: number | null;
	p95_ms: number | null;
	/** Distinct non-bot visitors over the range (a visitor is unique per day). */
	visitors: number;
};

export type StatusCount = { status: number; requests: number };

export type TopEntry = {
	/** Host, path, application id or dimension value, depending on the list. */
	key: string;
	requests: number;
	errors: number;
	bytes_out: number | null;
	avg_ms: number | null;
};

export type Analytics = {
	range: AnalyticsRange;
	bucket_seconds: number;
	series: AnalyticsPoint[];
	totals: AnalyticsTotals;
	statuses: StatusCount[];
	hosts: TopEntry[];
	paths: TopEntry[];
	/** Organization view only. */
	applications: TopEntry[];
	methods: TopEntry[];
	protocols: TopEntry[];
	/** Browser family, `bot`, `tool` (curl, SDKs), `other` or `unknown`. */
	clients: TopEntry[];
	/** Referer hosts other than the site itself. */
	referrers: TopEntry[];
	/** Every country with traffic, busiest first. */
	countries: CountryEntry[];
	/** The 100 most recently seen client IPs. */
	recent_visitors: VisitorEntry[];
};

export type CountryEntry = {
	/** ISO 3166-1 alpha-2. */
	code: string;
	requests: number;
	/** Distinct non-bot visitors (unique per day). */
	visitors: number;
};

export type VisitorEntry = {
	ip: string;
	/** Organization view only. */
	application_id?: string;
	country: string | null;
	region: string | null;
	city: string | null;
	/** Browser family, `bot`, `tool`, `other` or `unknown`. */
	client: string;
	/** `ios`, `android`, `windows`, `macos`, `linux`, `chromeos`, `other` or `unknown`. */
	os: string;
	requests: number;
	errors: number;
	bytes_out: number;
	/** Epoch milliseconds, minute precision. */
	first_seen: number;
	last_seen: number;
};

export type Sparklines = {
	bucket_seconds: number;
	since: number;
	/** Application id -> requests per bucket over the last 24h; apps without traffic are absent. */
	applications: Record<string, number[]>;
};

const org = (organizationId: string) =>
	`/api/v1/organizations/${encodeURIComponent(organizationId)}`;

export async function getOrganizationAnalytics(
	organizationId: string,
	range: AnalyticsRange,
): Promise<Analytics> {
	return (await api<Analytics>(`${org(organizationId)}/analytics?range=${range}`)).data!;
}

export async function getApplicationAnalytics(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	range: AnalyticsRange,
): Promise<Analytics> {
	const path = `${org(organizationId)}/projects/${encodeURIComponent(projectId)}/environments/${encodeURIComponent(
		environmentId,
	)}/applications/${encodeURIComponent(applicationId)}/analytics?range=${range}`;
	return (await api<Analytics>(path)).data!;
}

/** One request per page for every card's 24h requests chart. */
export async function getRequestSparklines(organizationId: string): Promise<Sparklines> {
	return (await api<Sparklines>(`${org(organizationId)}/analytics/sparklines`)).data!;
}

/** Time span of bucket `i`, e.g. "14:30–15:00". */
export function sparklineTime(sparklines: Sparklines, locale: string): (i: number) => string {
	const time = new Intl.DateTimeFormat(locale, { hour: '2-digit', minute: '2-digit' });
	const bucket = sparklines.bucket_seconds * 1000;
	return (i) => {
		const from = sparklines.since + i * bucket;
		return `${time.format(from)}–${time.format(from + bucket)}`;
	};
}

/** Element-wise sum of the given applications' sparklines (empty when none has traffic). */
export function sumSparklines(sparklines: Sparklines | null, applicationIds: string[]): number[] {
	const series = applicationIds
		.map((id) => sparklines?.applications[id])
		.filter((values): values is number[] => !!values);
	if (series.length === 0) return [];
	return series[0].map((_, index) => series.reduce((sum, values) => sum + (values[index] ?? 0), 0));
}

/** 1234 -> "1.2k". */
export function compactNumber(value: number, locale?: string): string {
	return new Intl.NumberFormat(locale, { notation: 'compact', maximumFractionDigits: 1 }).format(
		value,
	);
}

export function formatLatency(ms: number | null): string {
	if (ms === null || !Number.isFinite(ms)) return '-';
	return ms >= 1000 ? `${(ms / 1000).toFixed(ms >= 10_000 ? 0 : 1)} s` : `${Math.round(ms)} ms`;
}
