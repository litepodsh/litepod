import { api } from './base-client';

export type RuntimeStatus = {
	server_time: string;
	started_at: string;
	utc_offset_seconds: number;
	host_os: string;
	host_version: string | null;
	host_kernel_version: string | null;
	host_arch: string;
	host_distribution: string | null;
	podman_host_os: string | null;
	podman_host_arch: string | null;
};

export async function getRuntimeStatus(): Promise<RuntimeStatus> {
	const response = await api<RuntimeStatus>('/api/v1/runtime');
	return response.data!;
}
