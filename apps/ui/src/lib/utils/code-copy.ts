import { mount, unmount, type Component } from 'svelte';
import MonacoCodeBlock from '#lib/components/assistant/monaco-code-block.svelte';
import CopyButton from '#lib/components/custom/copy-button.svelte';

/**
 * Enhances every `<pre>` inside `root` with a read-only Monaco editor and a
 * copy button. Reuses both components to keep the chat consistent with the
 * other Monaco-bearing surfaces in the app.
 *
 * The markdown renderer emits raw sanitised HTML, so the Svelte components
 * are mounted imperatively rather than baked into the markup.
 */

const LANGUAGE_ALIASES: Record<string, string> = {
	yml: 'yaml',
	sh: 'shell',
	bash: 'shell',
	zsh: 'shell',
	fish: 'shell',
	nu: 'shell',
	powershell: 'powershell',
	ps1: 'powershell',
	js: 'javascript',
	jsx: 'javascript',
	ts: 'typescript',
	tsx: 'typescript',
	mjs: 'javascript',
	cjs: 'javascript',
	py: 'python',
	md: 'markdown',
	text: 'plaintext',
	txt: 'plaintext',
	dockerfile: 'dockerfile',
	'': 'plaintext',
};

function normalizeLanguage(raw: string | undefined): string {
	if (!raw) return 'plaintext';
	const lower = raw.toLowerCase();
	return LANGUAGE_ALIASES[lower] ?? lower;
}

interface Mounted {
	wrapper: HTMLDivElement;
	instances: Array<{ host: HTMLElement; instance: ReturnType<typeof mount> }>;
}

export function enhanceCodeBlocks(root: HTMLElement) {
	const mounted: Mounted[] = [];

	const attach = () => {
		const pres = root.querySelectorAll<HTMLPreElement>('pre');
		pres.forEach((pre) => {
			if (pre.dataset.codeAttached === '1') return;
			pre.dataset.codeAttached = '1';

			const codeEl = pre.querySelector('code');
			const code = codeEl?.textContent ?? pre.textContent ?? '';
			const langClass = Array.from(codeEl?.classList ?? []).find((c) => c.startsWith('language-'));
			const lang = normalizeLanguage(langClass?.replace(/^language-/, ''));

			const wrapper = document.createElement('div');
			wrapper.className = 'assistant-code-block relative my-2';
			pre.replaceWith(wrapper);

			const codeHost = document.createElement('div');
			wrapper.appendChild(codeHost);

			const btnHost = document.createElement('div');
			btnHost.className = 'absolute top-1 right-1 z-10';
			wrapper.appendChild(btnHost);

			const codeInstance = mount(MonacoCodeBlock as Component, {
				target: codeHost,
				props: {
					value: code,
					language: lang,
					maxHeight: '14rem',
					class: 'rounded-md',
				},
			});
			const copyInstance = mount(CopyButton as Component, {
				target: btnHost,
				props: {
					value: code,
					label: null,
					revealOnHover: true,
					class: 'size-7',
				},
			});

			mounted.push({
				wrapper,
				instances: [
					{ host: codeHost, instance: codeInstance },
					{ host: btnHost, instance: copyInstance },
				],
			});
		});
	};

	attach();

	// Streaming replaces the bubble contents as the assistant's text chunks
	// arrive; the MutationObserver re-attaches so newly mounted `<pre>`s also
	// become enhanced. The `data-code-attached` flag keeps this idempotent.
	const observer = new MutationObserver(() => queueMicrotask(attach));
	observer.observe(root, { childList: true, subtree: true });

	return {
		destroy() {
			observer.disconnect();
			for (const entry of mounted) {
				for (const sub of entry.instances) {
					try {
						unmount(sub.instance);
					} catch {
						// Already torn down by Svelte's own cleanup; nothing to do.
					}
				}
				entry.wrapper.remove();
			}
			mounted.length = 0;
		},
	};
}
