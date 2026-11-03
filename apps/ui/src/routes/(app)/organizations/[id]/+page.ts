import { redirect } from '@sveltejs/kit';

const MOVED_TO_INTEGRATIONS = ['git-providers', 'assistant', 'mcp', 'notifications'];

// These sections have their own page now. Old `?tab=` links keep landing on the same section.
export function load({ url }) {
	const tab = url.searchParams.get('tab');
	if (tab && MOVED_TO_INTEGRATIONS.includes(tab))
		redirect(307, `${url.pathname}/integrations${url.search}`);
}
