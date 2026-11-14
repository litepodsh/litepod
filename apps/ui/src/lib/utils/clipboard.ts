/**
 * Copies `value` to the clipboard, returning whether it worked.
 *
 * `navigator.clipboard` only exists in secure contexts (HTTPS, localhost), so a
 * UI served over plain HTTP on a LAN address has no Clipboard API at all and
 * every `navigator.clipboard.writeText` throws synchronously. The `execCommand`
 * fallback still copies there, as long as it runs inside a user gesture.
 */
export async function copyText(value: string): Promise<boolean> {
	if (navigator.clipboard?.writeText) {
		try {
			await navigator.clipboard.writeText(value);
			return true;
		} catch {
			// Insecure origin or denied permission: try the legacy path.
		}
	}

	return legacyCopy(value);
}

function legacyCopy(value: string): boolean {
	const active = document.activeElement instanceof HTMLElement ? document.activeElement : null;
	const area = document.createElement('textarea');
	area.value = value;
	area.setAttribute('readonly', '');
	area.style.position = 'fixed';
	area.style.top = '0';
	area.style.left = '0';
	area.style.width = '1px';
	area.style.height = '1px';
	area.style.opacity = '0';
	area.style.pointerEvents = 'none';
	document.body.appendChild(area);

	let copied = false;
	try {
		area.select();
		area.setSelectionRange(0, value.length);
		copied = document.execCommand('copy');
	} catch {
		copied = false;
	} finally {
		area.remove();
		// The hidden textarea steals focus; put it back so typing keeps working
		// in terminals and inline editors.
		active?.focus({ preventScroll: true });
	}

	return copied;
}
