/**
 * Example data for surfaces the API cannot back yet (deploy steps,
 * terminal sessions). Never sent to the API and
 * always rendered next to <PreviewBadge>. Turn it off with VITE_PREVIEW_DATA=false.
 */
export const PREVIEW_ENABLED = import.meta.env.VITE_PREVIEW_DATA !== 'false';

export function deploymentSteps(kind: 'compose' | 'git' | 'image'): string[] {
	if (kind === 'git') return ['Clone', 'Build', 'Start', 'Health', 'Route'];
	return ['Prepare', 'Pull', 'Start', 'Health', 'Route'];
}
