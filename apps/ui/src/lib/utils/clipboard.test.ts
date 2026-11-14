import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { copyText } from './clipboard.js';

describe('copyText', () => {
	beforeEach(() => {
		document.body.innerHTML = '';
	});

	afterEach(() => {
		vi.restoreAllMocks();
		Reflect.deleteProperty(navigator, 'clipboard');
		Reflect.deleteProperty(document, 'execCommand');
	});

	function setClipboard(value: Partial<Clipboard> | undefined) {
		Object.defineProperty(navigator, 'clipboard', { value, configurable: true });
	}

	// jsdom does not implement `document.execCommand` at all.
	function stubExecCommand(impl: (command: string) => boolean) {
		const exec = vi.fn(impl);
		Object.defineProperty(document, 'execCommand', { value: exec, configurable: true });
		return exec;
	}

	it('uses the Clipboard API when it exists', async () => {
		const writeText = vi.fn().mockResolvedValue(undefined);
		setClipboard({ writeText } as unknown as Clipboard);
		const exec = stubExecCommand(() => true);

		await expect(copyText('hello')).resolves.toBe(true);
		expect(writeText).toHaveBeenCalledWith('hello');
		expect(exec).not.toHaveBeenCalled();
	});

	it('falls back to execCommand on an insecure origin (no Clipboard API)', async () => {
		setClipboard(undefined);
		const exec = stubExecCommand((command) => {
			expect(command).toBe('copy');
			expect(document.querySelector('textarea')?.value).toBe('hello');
			return true;
		});

		await expect(copyText('hello')).resolves.toBe(true);
		expect(exec).toHaveBeenCalledOnce();
		expect(document.querySelector('textarea')).toBeNull();
	});

	it('falls back when the Clipboard API rejects (permission denied)', async () => {
		const writeText = vi.fn().mockRejectedValue(new Error('denied'));
		setClipboard({ writeText } as unknown as Clipboard);
		const exec = stubExecCommand(() => true);

		await expect(copyText('hello')).resolves.toBe(true);
		expect(exec).toHaveBeenCalledOnce();
	});

	it('reports failure when both paths fail', async () => {
		setClipboard(undefined);
		stubExecCommand(() => false);

		await expect(copyText('hello')).resolves.toBe(false);
	});
});
