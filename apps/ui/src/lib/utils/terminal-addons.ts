import { LigaturesAddon } from '@xterm/addon-ligatures';
import { SearchAddon } from '@xterm/addon-search';
import { Unicode11Addon } from '@xterm/addon-unicode11';
import type { Terminal } from '@xterm/xterm';

export function loadTerminalAddons(terminal: Terminal): SearchAddon {
	terminal.loadAddon(new Unicode11Addon());
	terminal.unicode.activeVersion = '11';
	terminal.loadAddon(new LigaturesAddon());

	const search = new SearchAddon({ highlightLimit: 1_000 });
	terminal.loadAddon(search);
	return search;
}
