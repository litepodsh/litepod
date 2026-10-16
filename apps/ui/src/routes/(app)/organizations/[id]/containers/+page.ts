import { redirect } from '@sveltejs/kit';
import type { PageLoad } from './$types';
import { setSelectedOrganizationId } from '#lib/utils/selected-organization.js';

// Lives in the System hub now, which works on the selected organization.
export const load: PageLoad = ({ params, url }) => {
	setSelectedOrganizationId(params.id);
	const query = new URLSearchParams({ tab: 'containers' });
	const server = url.searchParams.get('server');
	if (server) query.set('server', server);
	const view = url.searchParams.get('tab');
	if (view) query.set('view', view);
	redirect(307, `/system?${query}`);
};
