export type SidebarSide = 'left' | 'right';
const KEY = 'lp-sidebar-side';

function read(): SidebarSide {
	try {
		return localStorage.getItem(KEY) === 'right' ? 'right' : 'left';
	} catch {
		return 'left';
	}
}

export const sidebarSide = $state<{ current: SidebarSide }>({ current: typeof localStorage === 'undefined' ? 'left' : read() });

export function setSidebarSide(side: SidebarSide) {
	sidebarSide.current = side;
	try {
		localStorage.setItem(KEY, side);
	} catch {
		// Private mode: the choice lasts for this tab only.
	}
}
