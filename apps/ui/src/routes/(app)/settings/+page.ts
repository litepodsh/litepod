import { redirect } from '@sveltejs/kit';

// Instance settings live in the System hub now; the organization list has its
// own page. Old `?tab=` links keep landing on the same section.
export function load({ url }) {
	const tab = url.searchParams.get('tab');
	if (tab === 'organizations') redirect(307, '/organizations');
	const known = ['general', 'certificates', 'builds', 'rate-limit'];
	redirect(307, `/system?tab=${tab && known.includes(tab) ? tab : 'general'}`);
}
