// Mirrors `api/src/deployment/compose/domain_markers.rs`. Unlike secret markers, these are
// inserted literally: the API rewrites them with the route's address on every deploy.
const DOMAIN_MARKER = /^\{\{\s*(url|domain)(:[A-Za-z0-9._-]+)?(:\d+)?\s*\}\}$/;

export function isDomainMarker(text: string): boolean {
	return DOMAIN_MARKER.test(text.trim());
}

export function domainMarkerPresets(ports: number[]): string[] {
	return [
		'{{url}}',
		'{{domain}}',
		...ports.flatMap((port) => [`{{url:${port}}}`, `{{domain:${port}}}`]),
	];
}

/** Syntax reference shown next to the secret markers; `key` is the i18n message. */
export const DOMAIN_MARKER_DOCS: { syntax: string; key: string }[] = [
	{ syntax: '{{url}}', key: 'domain_marker_url' },
	{ syntax: '{{domain}}', key: 'domain_marker_domain' },
	{ syntax: '{{url:PORT}}', key: 'domain_marker_port' },
	{ syntax: '{{url:SERVICE:PORT}}', key: 'domain_marker_service' },
];
