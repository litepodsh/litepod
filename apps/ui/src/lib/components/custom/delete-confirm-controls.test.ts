import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, expect, it, vi } from 'vitest';
import HoldToDelete from './hold-to-delete.svelte';
import SlideToDelete from './slide-to-delete.svelte';
import DeleteConfirmControl from './delete-confirm-control.svelte';
import { setLocale } from '#lib/i18n/index.svelte.js';

afterEach(cleanup);

it('only confirms a hold after its full duration', async () => {
	vi.useFakeTimers();
	const onconfirm = vi.fn();
	render(HoldToDelete, { holdMs: 100, onconfirm });
	const button = screen.getByRole('button', { name: 'Hold to delete' });

	await fireEvent.pointerDown(button);
	vi.advanceTimersByTime(99);
	await fireEvent.pointerUp(button);
	expect(onconfirm).not.toHaveBeenCalled();

	await fireEvent.pointerDown(button);
	vi.advanceTimersByTime(100);
	expect(onconfirm).toHaveBeenCalledOnce();
	vi.useRealTimers();
});

it('confirms a held Enter key only after its full duration', async () => {
	vi.useFakeTimers();
	const onconfirm = vi.fn();
	render(HoldToDelete, { holdMs: 100, onconfirm });
	const button = screen.getByRole('button', { name: 'Hold to delete' });

	await fireEvent.keyDown(button, { key: 'Enter' });
	vi.advanceTimersByTime(99);
	await fireEvent.keyUp(button, { key: 'Enter' });
	expect(onconfirm).not.toHaveBeenCalled();

	await fireEvent.keyDown(button, { key: 'Enter' });
	vi.advanceTimersByTime(100);
	expect(onconfirm).toHaveBeenCalledOnce();
	vi.useRealTimers();
});

it('confirms a held Space key only after its full duration', async () => {
	vi.useFakeTimers();
	const onconfirm = vi.fn();
	render(HoldToDelete, { holdMs: 100, onconfirm });
	const button = screen.getByRole('button', { name: 'Hold to delete' });

	await fireEvent.keyDown(button, { key: ' ' });
	vi.advanceTimersByTime(100);
	expect(onconfirm).toHaveBeenCalledOnce();
	await fireEvent.keyUp(button, { key: ' ' });
	vi.useRealTimers();
});

it('ignores a held key outside the confirm keys', async () => {
	vi.useFakeTimers();
	const onconfirm = vi.fn();
	render(HoldToDelete, { holdMs: 100, onconfirm });
	const button = screen.getByRole('button', { name: 'Hold to delete' });

	await fireEvent.keyDown(button, { key: 'a' });
	vi.advanceTimersByTime(500);
	expect(onconfirm).not.toHaveBeenCalled();
	vi.useRealTimers();
});

it('takes focus when its dialog focuses the container', async () => {
	const onconfirm = vi.fn();
	const dialog = document.createElement('div');
	dialog.setAttribute('role', 'alertdialog');
	document.body.appendChild(dialog);

	render(HoldToDelete, { props: { onconfirm }, target: dialog });
	const button = screen.getByRole('button', { name: 'Hold to delete' });
	expect(document.activeElement).not.toBe(button);

	await fireEvent.focusIn(dialog);
	expect(document.activeElement).toBe(button);
});

it('does not steal focus outside a dialog', () => {
	const onconfirm = vi.fn();
	render(HoldToDelete, { onconfirm });
	const button = screen.getByRole('button', { name: 'Hold to delete' });
	expect(document.activeElement).not.toBe(button);
});

it('only confirms a slide that reaches the end of its track', async () => {
	const onconfirm = vi.fn();
	const { container } = render(SlideToDelete, { onconfirm });
	const track = container.querySelector('[data-slot="slide-to-delete"] > div') as HTMLDivElement;
	Object.defineProperty(track, 'clientWidth', { value: 280 });
	track.getBoundingClientRect = () => ({ left: 0 }) as DOMRect;
	const knob = screen.getByRole('button', { name: 'Slide to delete' });
	knob.setPointerCapture = vi.fn();

	await fireEvent.pointerDown(knob, { pointerId: 1, clientX: 26 });
	await fireEvent.pointerUp(knob, { pointerId: 1, clientX: 120 });
	expect(onconfirm).not.toHaveBeenCalled();

	await fireEvent.pointerDown(knob, { pointerId: 2, clientX: 26 });
	await fireEvent.pointerUp(knob, { pointerId: 2, clientX: 276 });
	expect(onconfirm).toHaveBeenCalledOnce();
});

it('uses a concise Spanish hold label', () => {
	vi.stubGlobal(
		'matchMedia',
		vi.fn().mockReturnValue({
			matches: false,
			addEventListener: vi.fn(),
			removeEventListener: vi.fn(),
		}),
	);
	setLocale('es');
	render(DeleteConfirmControl, { onconfirm: vi.fn() });
	expect(screen.getByRole('button', { name: 'Mantén pulsado' })).toBeTruthy();
	setLocale('en');
});
