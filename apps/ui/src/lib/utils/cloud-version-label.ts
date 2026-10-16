export function cloudVersionLabel(version: string, isCloud: boolean): string {
	return isCloud ? `${version} — Cloud edition` : version;
}
