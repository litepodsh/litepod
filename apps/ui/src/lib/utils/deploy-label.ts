/**
 * "Deploy" before the first start, "Redeploy" once containers are live — the
 * two actions look identical in the UI but the second one recreates running
 * containers, so the label is the only warning the user gets up front.
 */
export function deployLabel(running: boolean, subject?: string): string {
	const verb = running ? 'Redeploy' : 'Deploy';
	return subject ? `${verb} ${subject}` : verb;
}
