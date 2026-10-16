import { API_URL, api } from './base-client';

export type MetricsRange = '1h' | '6h' | '24h' | '7d' | '30d' | '90d' | '365d' | 'all';
/** Every preset, for the full history explorer. */
export const HISTORY_RANGES: MetricsRange[] = ['1h', '24h', '7d', '30d', '90d', '365d', 'all'];
/** A preset ending now, or an explicit `[from, to)` window in epoch ms. */
export type MetricsWindow = MetricsRange | { from: number; to: number };

export type MetricsHistoryPoint = {
	/** Bucket start, epoch milliseconds. */
	ts: number;
	cpu_avg: number | null;
	cpu_max: number | null;
	mem_avg: number | null;
	mem_limit: number | null;
};

export type ContainerHistory = {
	container_name: string;
	points: MetricsHistoryPoint[];
};

/** Host usage per bucket; totals are the capacity of that moment, so a resize shows. */
export type HostHistoryPoint = {
	ts: number;
	disk_used: number | null;
	disk_total: number | null;
	mem_used: number | null;
	mem_total: number | null;
	cpu_cores: number | null;
};

export type MetricsHistory = {
	range: MetricsRange;
	/** Window covered, epoch ms (`to` exclusive). */
	from: number;
	to: number;
	bucket_seconds: number;
	containers: ContainerHistory[];
	/** Empty for an application. */
	host: HostHistoryPoint[];
};

function windowQuery(window: MetricsWindow): string {
	return typeof window === 'string'
		? `range=${window}`
		: `from=${Math.round(window.from)}&to=${Math.round(window.to)}`;
}

export type MetricsRetention = {
	retention_days: number;
	default_days: number;
	max_days: number;
};

export async function getApplicationMetricsHistory(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
	window: MetricsWindow,
): Promise<MetricsHistory> {
	const path = `/api/v1/organizations/${encodeURIComponent(organizationId)}/projects/${encodeURIComponent(
		projectId,
	)}/environments/${encodeURIComponent(environmentId)}/applications/${encodeURIComponent(
		applicationId,
	)}/containers/metrics/history?${windowQuery(window)}`;
	return (await api<MetricsHistory>(path)).data!;
}

export async function getServerMetricsHistory(
	organizationId: string,
	serverId: string,
	window: MetricsWindow,
): Promise<MetricsHistory> {
	const path = `/api/v1/organizations/${encodeURIComponent(organizationId)}/servers/${encodeURIComponent(
		serverId,
	)}/podman/metrics/history?${windowQuery(window)}`;
	return (await api<MetricsHistory>(path)).data!;
}

export async function getMetricsRetention(organizationId: string): Promise<MetricsRetention> {
	return (
		await api<MetricsRetention>(
			`/api/v1/organizations/${encodeURIComponent(organizationId)}/metrics/retention`,
		)
	).data!;
}

export async function updateMetricsRetention(
	organizationId: string,
	retentionDays: number,
): Promise<MetricsRetention> {
	return (
		await api<MetricsRetention>(
			`/api/v1/organizations/${encodeURIComponent(organizationId)}/metrics/retention`,
			{ method: 'PUT', body: JSON.stringify({ retention_days: retentionDays }) },
		)
	).data!;
}

/** Sums every container per bucket: the application-wide CPU and memory series. */
export function totalSeries(history: MetricsHistory): {
	ts: number[];
	cpuAvg: number[];
	cpuMax: number[];
	memAvg: number[];
} {
	const buckets = new Map<number, { cpuAvg: number; cpuMax: number; memAvg: number }>();
	for (const container of history.containers) {
		for (const point of container.points) {
			const bucket = buckets.get(point.ts) ?? { cpuAvg: 0, cpuMax: 0, memAvg: 0 };
			bucket.cpuAvg += point.cpu_avg ?? 0;
			bucket.cpuMax += point.cpu_max ?? 0;
			bucket.memAvg += point.mem_avg ?? 0;
			buckets.set(point.ts, bucket);
		}
	}
	const ts = [...buckets.keys()].sort((a, b) => a - b);
	return {
		ts,
		cpuAvg: ts.map((key) => buckets.get(key)!.cpuAvg),
		cpuMax: ts.map((key) => buckets.get(key)!.cpuMax),
		memAvg: ts.map((key) => buckets.get(key)!.memAvg),
	};
}

export type ContainerMetricState = 'running' | 'not_running' | 'unavailable';

export type ContainerMetric = {
	id: string;
	name: string;
	state: ContainerMetricState;
	cpu_percent: number | null;
	memory_usage_bytes: number | null;
	memory_limit_bytes: number | null;
	observed_at: string;
};

export type ContainerMetricsSnapshot = {
	observed_at: string;
	metrics: ContainerMetric[];
};

export function streamApplicationContainerMetrics(
	organizationId: string,
	projectId: string,
	environmentId: string,
	applicationId: string,
): EventSource {
	const path = `/api/v1/organizations/${encodeURIComponent(organizationId)}/projects/${encodeURIComponent(
		projectId,
	)}/environments/${encodeURIComponent(environmentId)}/applications/${encodeURIComponent(
		applicationId,
	)}/containers/metrics/stream`;
	return new EventSource(`${API_URL}${path}`, { withCredentials: true });
}

export function metricIndex(snapshot: ContainerMetricsSnapshot): Map<string, ContainerMetric> {
	const indexed = new Map<string, ContainerMetric>();
	for (const metric of snapshot.metrics) {
		indexed.set(metric.id, metric);
		indexed.set(metric.name, metric);
	}
	return indexed;
}

export function metricContainerIds(
	container: { container_name?: string | null } | null,
	composeServices: { container_name?: string | null; status: string }[],
): string[] {
	const compose = composeServices
		.filter((service) =>
			['running', 'up'].some((value) => service.status.toLowerCase().startsWith(value)),
		)
		.map((service) => service.container_name?.trim() ?? '')
		.filter(Boolean);
	const standard = container?.container_name?.trim();
	return [...new Set(compose.length > 0 ? compose : standard ? [standard] : [])];
}

export function formatCpu(value: number | null): string {
	return value === null || !Number.isFinite(value) ? 'Unavailable' : `${value.toFixed(1)}%`;
}

export function formatMemory(used: number | null, limit: number | null): string {
	if (used === null) return 'Unavailable';
	return `${formatBytes(used)}${limit === null ? '' : ` / ${formatBytes(limit)}`}`;
}

export function formatBytes(bytes: number): string {
	const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
	let value = bytes;
	let unit = 0;
	while (value >= 1024 && unit < units.length - 1) {
		value /= 1024;
		unit += 1;
	}
	const rounded = value >= 10 || Number.isInteger(value) ? value.toFixed(0) : value.toFixed(1);
	return `${rounded} ${units[unit]}`;
}

/** A history laid on its bucket grid: one column per bucket, null where nothing was stored. */
export type HistoryGrid = {
	/** Start of each column, epoch ms. */
	starts: number[];
	bucketMs: number;
	cpu: (number | null)[];
	/** Host RAM when the server reported it, otherwise the containers' sum. */
	mem: (number | null)[];
	memCap: (number | null)[];
	disk: (number | null)[];
	diskCap: (number | null)[];
	/** True when `mem` is the host's own usage rather than its containers'. */
	hostMemory: boolean;
};

export function historyGrid(history: MetricsHistory): HistoryGrid {
	const bucketMs = history.bucket_seconds * 1000;
	const cols = Math.max(1, Math.ceil((history.to - history.from) / bucketMs));
	const starts = Array.from({ length: cols }, (_, i) => history.from + i * bucketMs);
	const empty = () => Array<number | null>(cols).fill(null);
	const slot = (ts: number) => {
		const i = Math.floor((ts - history.from) / bucketMs);
		return i >= 0 && i < cols ? i : null;
	};
	const cpu = empty();
	const containerMem = empty();
	const totals = totalSeries(history);
	totals.ts.forEach((ts, k) => {
		const i = slot(ts);
		if (i === null) return;
		cpu[i] = totals.cpuAvg[k];
		containerMem[i] = totals.memAvg[k];
	});
	const hostMem = empty();
	const memCap = empty();
	const disk = empty();
	const diskCap = empty();
	for (const point of history.host) {
		const i = slot(point.ts);
		if (i === null) continue;
		hostMem[i] = point.mem_used;
		memCap[i] = point.mem_total;
		disk[i] = point.disk_used;
		diskCap[i] = point.disk_total;
	}
	const hostMemory = hostMem.some((v) => v !== null);
	return {
		starts,
		bucketMs,
		cpu,
		mem: hostMemory ? hostMem : containerMem,
		memCap,
		disk,
		diskCap,
		hostMemory,
	};
}
