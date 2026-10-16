import { beforeEach, describe, expect, it } from 'vitest';
import { assistantState } from '#lib/state/assistant.svelte.js';
import { askAboutLogs, thread } from './thread.svelte';
import { logQuestion } from './ui-tools';

const storage = new Map<string, string>();
Object.defineProperty(window, 'localStorage', {
	configurable: true,
	value: {
		getItem: (key: string) => storage.get(key) ?? null,
		setItem: (key: string, value: string) => storage.set(key, value),
		removeItem: (key: string) => storage.delete(key),
	},
});

describe('logQuestion', () => {
	it('includes the project, application, log kind, and log text', () => {
		const question = logQuestion('Payments', 'api', 'runtime', 'line one\nline two');

		expect(question).toContain('project `Payments`');
		expect(question).toContain('application `api`');
		expect(question).toContain('runtime');
		expect(question).toContain('line one\nline two');
	});
});

describe('askAboutLogs', () => {
	beforeEach(() => {
		assistantState.mode = 'nav';
		assistantState.view = 'history';
		thread.items = [];
		thread.running = false;
		thread.pendingDraft = null;
		thread.pendingDraftNotice = false;
	});

	it('opens an editable contextual draft without sending it', () => {
		askAboutLogs('Payments', 'api', 'runtime', 'line one\nline two');

		expect(assistantState.mode).toBe('chat');
		expect(assistantState.view).toBe('thread');
		expect(thread.pendingDraft).toContain('project `Payments`');
		expect(thread.pendingDraft).toContain('application `api`');
		expect(thread.pendingDraft).toContain('line one\nline two');
		expect(thread.pendingDraftNotice).toBe(true);
		expect(thread.items).toEqual([]);
		expect(thread.running).toBe(false);
	});
});
