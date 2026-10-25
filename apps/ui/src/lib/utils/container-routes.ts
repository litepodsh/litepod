type Loose = Record<string, unknown>;

/** Container-side TCP ports Podman lists for a container (published ones only). */
export function listedContainerPorts(entry: Loose): number[] {
	const raw = entry.Ports ?? entry.ports;
	if (!Array.isArray(raw)) return [];
	const found = new Set<number>();
	for (const port of raw) {
		if (typeof port !== 'object' || port === null) continue;
		const p = port as Loose;
		const proto = String(p.protocol ?? p.Type ?? 'tcp').toLowerCase();
		const container = Number(p.container_port ?? p.ContainerPort ?? p.PrivatePort);
		if (proto === 'tcp' && Number.isInteger(container) && container > 0) found.add(container);
	}
	return [...found].sort((a, b) => a - b);
}

/** Container-side TCP ports from an inspect record: exposed by the image plus published. */
export function inspectedContainerPorts(data: Loose): number[] {
	const found = new Set<number>();
	const add = (key: string) => {
		const match = /^(\d+)\/tcp$/.exec(key);
		if (match) found.add(Number(match[1]));
	};
	const exposed = ((data.Config as Loose | undefined)?.ExposedPorts ?? {}) as Loose;
	Object.keys(exposed).forEach(add);
	const published = ((data.NetworkSettings as Loose | undefined)?.Ports ?? {}) as Loose;
	Object.keys(published).forEach(add);
	return [...found].sort((a, b) => a - b);
}

/** Address other containers on a shared network use to reach this one. */
export function internalRoute(host: string, port: number): string {
	return `http://${host}:${port}`;
}

export type InternalIp = { network: string; ip: string };

/** Per-network IPs from an inspect record, skipping networks with no address. */
export function inspectedIps(data: Loose): InternalIp[] {
	const networks = ((data.NetworkSettings as Loose | undefined)?.Networks ?? {}) as Record<
		string,
		Loose
	>;
	return Object.entries(networks)
		.map(([network, value]) => ({
			network,
			ip: String(value?.IPAddress ?? ''),
		}))
		.filter((row) => row.ip);
}

/** Splits `registry/repo:tag` (or `repo@sha256:...`) so the tag can be set apart. */
export function splitImage(image: string): { repo: string; tag: string } {
	if (!image) return { repo: '', tag: '' };
	const digest = image.indexOf('@');
	if (digest > 0)
		return {
			repo: image.slice(0, digest),
			tag: image.slice(digest + 1, digest + 20),
		};
	const slash = image.lastIndexOf('/');
	const colon = image.lastIndexOf(':');
	if (colon > slash) return { repo: image.slice(0, colon), tag: image.slice(colon + 1) };
	return { repo: image, tag: 'latest' };
}

/** Compose project a container belongs to, or '' when it runs standalone. */
export function containerProject(entry: Loose): string {
	const labels = (entry.Labels ?? entry.labels ?? {}) as Record<string, unknown>;
	return String(labels['io.podman.compose.project'] ?? labels['com.docker.compose.project'] ?? '');
}
