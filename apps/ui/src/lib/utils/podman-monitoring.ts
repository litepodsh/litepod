export type UsageSort = 'cpu' | 'memory' | 'disk';

export type ContainerUsage = {
	id: string;
	name: string;
	cpu: number | null;
	memory: number | null;
	disk: number | null;
};

export function resourcePercent(
	used: number | undefined,
	total: number | undefined,
): number | null {
	if (used === undefined || total === undefined || total <= 0) return null;
	return Math.max(0, Math.min(100, (used / total) * 100));
}

export function containerSizeBytes(value: unknown): number | null {
	if (typeof value === 'number' && Number.isFinite(value)) return value;
	if (typeof value !== 'string') return null;
	const match = value.trim().match(/^([\d.]+)\s*(B|KB|KiB|MB|MiB|GB|GiB|TB|TiB)?$/i);
	if (!match) return null;
	const amount = Number(match[1]);
	if (!Number.isFinite(amount)) return null;
	const unit = (match[2] ?? 'B').toLowerCase();
	const multiplier = {
		b: 1,
		kb: 1e3,
		kib: 1024,
		mb: 1e6,
		mib: 1024 ** 2,
		gb: 1e9,
		gib: 1024 ** 3,
		tb: 1e12,
		tib: 1024 ** 4,
	}[unit];
	return multiplier === undefined ? null : amount * multiplier;
}

export function sortContainerUsage(items: ContainerUsage[], sort: UsageSort): ContainerUsage[] {
	return [...items].sort((left, right) => {
		const leftValue = left[sort];
		const rightValue = right[sort];
		if (leftValue === null) return rightValue === null ? left.name.localeCompare(right.name) : 1;
		if (rightValue === null) return -1;
		return rightValue - leftValue || left.name.localeCompare(right.name);
	});
}
