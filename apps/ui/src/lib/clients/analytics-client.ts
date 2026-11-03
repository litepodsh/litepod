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
	/** Named agents behind `bot` requests (Googlebot, GPTBot, ...), busiest first. */
	bots: TopEntry[];
	/** Referer hosts other than the site itself. */
	referrers: TopEntry[];
	/** Every country with traffic, busiest first. */
	countries: CountryEntry[];
	/** The 100 most recently seen client IPs. */
	recent_visitors: VisitorEntry[];
	/** Slowest by average response time, slowest first. */
	slow_hosts: SlowEntry[];
	slow_paths: SlowEntry[];
	/** Organization view only. */
	slow_applications: SlowEntry[];
};

export type SlowEntry = {
	/** Host, path or application id, depending on the list. */
	key: string;
	/** The path's application (organization view, `slow_paths` only). */
	application_id?: string;
	requests: number;
	avg_ms: number | null;
	p95_ms: number | null;
	max_ms: number | null;
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
	/** Named agent for bots and scripts (Googlebot, curl/8.7.1, ...); empty for browsers. */
	agent: string;
	requests: number;
	errors: number;
	bytes_out: number;
	/** Epoch milliseconds, minute precision. */
	first_seen: number;
	last_seen: number;
};

export type PathGroupEntry = {
	/** Country code, `Region, CC`, `City, CC`, client family or agent name. */
	key: string;
	requests: number;
	/** Distinct client IPs in this group. */
	ips: number;
};

/** One endpoint: how often it is called, by how many IPs, and from where. */
export type PathAnalytics = {
	path: string;
	bucket_seconds: number;
	series: { ts: number; requests: number; ips: number }[];
	requests: number;
	/** Server errors (5xx). */
	errors: number;
	avg_ms: number | null;
	p95_ms: number | null;
	/** Distinct client IPs over the range; a lower bound on very busy endpoints. */
	ips: number;
	countries: PathGroupEntry[];
	regions: PathGroupEntry[];
	cities: PathGroupEntry[];
	clients: PathGroupEntry[];
	agents: PathGroupEntry[];
	/** The busiest client IPs. */
	visitors: VisitorEntry[];
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

const pathQuery = (path: string, range: AnalyticsRange) =>
	`analytics/path?path=${encodeURIComponent(path)}&range=${range}`;

export async function getOrganizationPathAnalytics(
	organizationId: string,
	path: string,
	range: AnalyticsRange,
): Promise<PathAnalytics> {
	return (await api<PathAnalytics>(`${org(organizationId)}/${pathQuery(path, range)}`)).data!;
}

export async function getApplicationPathAnalytics(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	path: string,
	range: AnalyticsRange,
): Promise<PathAnalytics> {
	const application = `${org(organizationId)}/projects/${encodeURIComponent(projectId)}/environments/${encodeURIComponent(
		environmentId,
	)}/applications/${encodeURIComponent(applicationId)}`;
	return (await api<PathAnalytics>(`${application}/${pathQuery(path, range)}`)).data!;
}

/** What someone typed or pasted (`version.json`, a full URL) as a stored path; null if none. */
export function endpointPath(input: string): string | null {
	const text = input.trim();
	if (!text) return null;
	try {
		return new URL(text, 'http://path.invalid/').pathname;
	} catch {
		return null;
	}
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
