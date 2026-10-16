import type { ServerSummaryDto } from '#lib/clients/project-client.js';

export function resourceLimitGuidance(
	capacity: ServerSummaryDto['capacity'],
	baseline?: { cpu_cores: number; memory_bytes: number } | null,
) {
	const maxCpu =
		capacity.cpu_cores && Number.isFinite(capacity.cpu_cores)
			? Math.floor(capacity.cpu_cores * 10) / 10
			: null;
	const maxMemoryMib =
		capacity.memory_bytes && Number.isFinite(capacity.memory_bytes)
			? Math.floor(capacity.memory_bytes / 1048576 / 64) * 64
			: null;
	return {
		maxCpu: maxCpu && maxCpu >= 0.1 ? maxCpu : null,
		recommendedCpu:
			maxCpu && maxCpu >= 0.1 && baseline
				? Math.max(0.1, Math.floor((maxCpu - baseline.cpu_cores * 1.8) * 10) / 10)
				: null,
		maxMemoryMib: maxMemoryMib && maxMemoryMib >= 64 ? maxMemoryMib : null,
		recommendedMemoryMib:
			maxMemoryMib && maxMemoryMib >= 64 && baseline
				? Math.max(
						64,
						Math.floor((maxMemoryMib - (baseline.memory_bytes * 1.8) / 1048576) / 64) * 64,
					)
				: null,
	};
}
