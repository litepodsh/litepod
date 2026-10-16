/** Display names for the `ID=` values of `/etc/os-release` and Rust's `std::env::consts::OS`. */
export const OS_LABELS: Record<string, string> = {
	arch: 'Arch Linux',
	debian: 'Debian',
	fedora: 'Fedora',
	linux: 'Linux',
	macos: 'macOS',
	omarchy: 'Omarchy',
	rhel: 'Red Hat Enterprise Linux',
	ubuntu: 'Ubuntu',
	windows: 'Windows',
};

/** Falls back to the raw id so an unmapped distro still reads as itself. */
export function osLabel(id: string | null | undefined, version?: string | null): string | null {
	if (!id) return null;
	const name = OS_LABELS[id] ?? id;
	return version ? `${name} ${version}` : name;
}
