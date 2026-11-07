import { afterEach, describe, expect, it, vi } from 'vitest';
import { pollWhileVisible } from './poll';

function setHidden(hidden: boolean) {
	Object.defineProperty(document, 'hidden', { configurable: true, get: () => hidden });
	document.dispatchEvent(new Event('visibilitychange'));
}

describe('pollWhileVisible', () => {
	afterEach(() => {
		vi.useRealTimers();
		setHidden(false);
	});

	it('skips ticks while hidden, refreshes on return and stops on cleanup', () => {
		vi.useFakeTimers();
		const fn = vi.fn();
		const stop = pollWhileVisible(fn, 1000);

		vi.advanceTimersByTime(1000);
		expect(fn).toHaveBeenCalledTimes(1);

		setHidden(true);
		vi.advanceTimersByTime(5000);
		expect(fn).toHaveBeenCalledTimes(1);

		setHidden(false);
		expect(fn).toHaveBeenCalledTimes(2);

		stop();
		vi.advanceTimersByTime(5000);
		setHidden(false);
		expect(fn).toHaveBeenCalledTimes(2);
	});
});
