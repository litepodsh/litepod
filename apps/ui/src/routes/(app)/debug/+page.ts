import { redirect } from '@sveltejs/kit';

// Lives in the System hub now.
export function load() {
	redirect(307, '/system?tab=debug');
}
