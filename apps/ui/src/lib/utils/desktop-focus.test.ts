import { afterEach, describe, expect, it, vi } from 'vitest';
import { focusOnDesktop, hasPrecisePointer } from './desktop-focus.js';

function stubPointer(matches: boolean) {
	vi.stubGlobal('matchMedia', () => ({ matches }));
}

afterEach(() => {
	vi.unstubAllGlobals();
	(document.activeElement as HTMLElement | null)?.blur();
});

describe('hasPrecisePointer', () => {
	it('is true for mouse or trackpad devices', () => {
		stubPointer(true);
		expect(hasPrecisePointer()).toBe(true);
	});

	it('is false for touch devices', () => {
		stubPointer(false);
		expect(hasPrecisePointer()).toBe(false);
	});
});

describe('focusOnDesktop', () => {
	it('focuses the field on desktop', () => {
		stubPointer(true);
		const input = document.createElement('input');
		document.body.append(input);

		expect(focusOnDesktop(input)).toBe(true);
		expect(document.activeElement).toBe(input);
	});

	it('leaves the field alone on touch devices', () => {
		stubPointer(false);
		const input = document.createElement('input');
		document.body.append(input);

		expect(focusOnDesktop(input)).toBe(false);
		expect(document.activeElement).not.toBe(input);
	});

	it('does not steal focus while the user is typing elsewhere', () => {
		stubPointer(true);
		const field = document.createElement('input');
		const other = document.createElement('input');
		document.body.append(field, other);
		other.focus();

		expect(focusOnDesktop(field)).toBe(false);
		expect(document.activeElement).toBe(other);
	});

	it('does not steal focus while a dialog is open', () => {
		stubPointer(true);
		const field = document.createElement('input');
		const dialog = document.createElement('div');
		const inside = document.createElement('button');
		dialog.setAttribute('role', 'dialog');
		dialog.append(inside);
		document.body.append(field, dialog);
		inside.focus();

		expect(focusOnDesktop(field)).toBe(false);
		expect(document.activeElement).toBe(inside);
	});

	it('takes focus from a link, like the sidebar entry that opened the page', () => {
		stubPointer(true);
		const field = document.createElement('input');
		const link = document.createElement('a');
		link.href = '#';
		document.body.append(field, link);
		link.focus();

		expect(focusOnDesktop(field)).toBe(true);
		expect(document.activeElement).toBe(field);
	});

	it('ignores a missing element', () => {
		stubPointer(true);
		expect(focusOnDesktop(null)).toBe(false);
	});
});
