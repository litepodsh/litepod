import { redirect } from '@sveltejs/kit';

// Billing moved to its own main tab; keep old links and Polar return URLs working.
export function load({ url }) {
	const target = new URL('/billing', url);
	target.search = url.search;
	redirect(308, `${target.pathname}${target.search}`);
}
