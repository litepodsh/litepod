import { api } from './base-client';
import type { ApplicationDto } from './project-client';

export type CatalogPort = {
	container: number;
	description: string;
	primary: boolean;
	service?: string | null;
};
export type CatalogFile = { filename: string; content: string };
export type CatalogSearchInput = { q?: string; category?: string; limit?: number };
export type CatalogTemplate = {
	id: string;
	name: string;
	description: string;
	version: string;
	categories: string[];
	tags: string[];
	ports: CatalogPort[];
	icon_url: string | null;
	ui_url: string | null;
	website: string | null;
	docs: string | null;
	source: string | null;
	license: string | null;
	/** The app does not fully work over plain http; generated sslip.io domains are http. */
	requires_https?: boolean;
	/** The app stores its URL internally on first boot. */
	url_persisted?: boolean;
	manifest?: CatalogFile;
	compose?: CatalogFile;
	env?: CatalogFile | null;
	setup?: CatalogFile | null;
};
export function catalogTemplatesPath(input: CatalogSearchInput = {}): string {
	const params = new URLSearchParams();
	if (input.q?.trim()) params.set('q', input.q.trim());
	if (input.category?.trim()) params.set('category', input.category.trim());
	if (input.limit !== undefined) params.set('limit', String(input.limit));
	const query = params.toString();
	return `/api/v1/catalog-templates${query ? `?${query}` : ''}`;
}
// Mirrors the catalog `ui` path. Used only where the template detail (and its
// resolved `ui_url`) is not loaded, e.g. an application that only stores its
// catalog_template_id.
export function catalogTemplateDocsUrl(templateId: string): string {
	return `https://templates.litepod.sh/api/v1/templates/${encodeURIComponent(templateId)}`;
}
export async function listCatalogTemplates(
	input: CatalogSearchInput = {},
): Promise<CatalogTemplate[]> {
	return (await api<CatalogTemplate[]>(catalogTemplatesPath(input))).data ?? [];
}
export async function getCatalogTemplate(id: string): Promise<CatalogTemplate> {
	return (await api<CatalogTemplate>(`/api/v1/catalog-templates/${encodeURIComponent(id)}`)).data!;
}
export async function createCatalogTemplateApplication(
	organizationId: string,
	projectId: string,
	environmentId: string,
	templateId: string,
	serverId: string,
): Promise<ApplicationDto> {
	return (
		await api<ApplicationDto>(
			`/api/v1/organizations/${organizationId}/projects/${projectId}/environments/${environmentId}/catalog-templates/${encodeURIComponent(templateId)}`,
			{ method: 'POST', body: JSON.stringify({ server_id: serverId }) },
		)
	).data!;
}
