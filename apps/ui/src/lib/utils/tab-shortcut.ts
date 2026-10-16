/** True on macOS/iOS, where the modifier key to show is ⌘ instead of Ctrl. */
export const isMacPlatform =
	typeof navigator !== 'undefined' && /Mac|iPhone|iPad|iPod/.test(navigator.userAgent);

/** How many tabs a row can address by shortcut: 1-9 plus 0 for the tenth. */
export const MAX_TAB_SHORTCUTS = 10;

/** The digit shown for a tab index, or undefined past the addressable range. */
export function tabShortcutDigit(index: number): number | undefined {
	if (index < 0 || index >= MAX_TAB_SHORTCUTS) return undefined;
	// Browsers use 0 for the tenth tab; follow the convention people already know.
	return index === 9 ? 0 : index + 1;
}

/** Ctrl/Cmd+1..9,0 tab switching, shared by every page/dialog using AnimatedTabs. */
export function tabIndexFromKeydown(event: KeyboardEvent, itemCount: number): number | null {
	if (!(event.metaKey || event.ctrlKey)) return null;
	if (!/^[0-9]$/.test(event.key)) return null;
	const digit = Number(event.key);
	const index = digit === 0 ? 9 : digit - 1;
	if (index >= itemCount) return null;
	return index;
}
