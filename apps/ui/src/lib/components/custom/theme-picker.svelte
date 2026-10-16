<script lang="ts" module>
	import type { Palette } from '#lib/state/palette.svelte.js';
	export type Mode = 'light' | 'dark';
	export const themeId = (palette: Palette, mode: Mode) => `${palette}-${mode}` as const;
</script>

<script lang="ts">
	import { setMode, userPrefersMode, mode } from 'mode-watcher';
	import SoftSwitch from '#lib/components/custom/soft-switch.svelte';
	import { paletteState, setPalette } from '#lib/state/palette.svelte.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import { cn } from '#lib/utils.js';

	const THEMES: [Palette, Mode][] = [
		['warm', 'light'],
		['warm', 'dark'],
		['cool', 'light'],
		['cool', 'dark'],
	];

	let system = $derived(userPrefersMode.current === 'system');
	let liveMode = $derived<Mode>(mode.current === 'dark' ? 'dark' : 'light');

	// With Match system on, a card only picks the palette; the OS keeps choosing light or dark.
	function pick(palette: Palette, m: Mode) {
		setPalette(palette);
		if (!system) setMode(m);
	}
</script>

<div class="grid grid-cols-2 gap-2.5" role="radiogroup" aria-label={t('menu.appearance.title')}>
	{#each THEMES as [p, m] (themeId(p, m))}
		{@const selected = paletteState.current === p && liveMode === m}
		{@const paired = system && paletteState.current === p && !selected}
		<button
			type="button"
			role="radio"
			aria-checked={selected}
			onclick={() => pick(p, m)}
			class={cn(
				'lp-raised flex flex-col gap-2 rounded-2xl border bg-card p-2 text-left transition-[border-color,box-shadow] duration-200 ease-(--lp-ease)',
				selected && 'border-primary ring-3 ring-primary/15',
				paired && 'border-dashed border-primary',
			)}>
			<span class="theme-preview" data-theme={themeId(p, m)} aria-hidden="true"></span>
			<span class="flex items-center justify-between px-1 text-sm font-medium">
				{t(`menu.appearance.${p}_${m}`)}
				{#if selected && system}
					<small class="font-mono text-[11px] text-muted-foreground lowercase"
						>{t('menu.appearance.system')}</small>
				{:else if paired}
					<small class="font-mono text-[11px] text-muted-foreground"
						>{t('menu.appearance.paired')}</small>
				{/if}
			</span>
		</button>
	{/each}
</div>
<div class="lp-inset mt-3 flex items-center justify-between gap-3 rounded-xl bg-secondary p-3">
	<span>
		<b class="block text-sm font-medium">{t('menu.appearance.match_system')}</b>
		<small class="block text-xs text-muted-foreground">
			{system
				? t('menu.appearance.match_system_on', {
						theme: t(`menu.appearance.${paletteState.current}_${liveMode}`),
					})
				: t('menu.appearance.match_system_off')}
		</small>
	</span>
	<SoftSwitch
		checked={system}
		onCheckedChange={(on) => setMode(on ? 'system' : liveMode)}
		aria-label={t('menu.appearance.match_system')} />
</div>

<style>
	/* Static colors: each card always shows its own theme, whatever is active. */
	.theme-preview {
		position: relative;
		display: block;
		height: 74px;
		overflow: hidden;
		border: 1px solid var(--pv-line);
		border-radius: 9px;
		background: var(--pv-bg);
	}
	.theme-preview::before {
		position: absolute;
		inset: 0 0 auto 0;
		height: 16px;
		border-bottom: 1px solid var(--pv-line);
		background: var(--pv-surface);
		content: '';
	}
	.theme-preview::after {
		position: absolute;
		top: 24px;
		left: 8px;
		width: 58%;
		height: 40px;
		border: 1px solid var(--pv-line);
		border-radius: 4px;
		background: var(--pv-surface);
		box-shadow: inset 8px 8px 0 -4px var(--pv-accent);
		content: '';
	}
	/* Phones show the picker inside the narrow user menu: shorter cards. */
	@media (max-width: 640px) {
		.theme-preview {
			height: 44px;
		}
		.theme-preview::before {
			height: 10px;
		}
		.theme-preview::after {
			top: 15px;
			height: 22px;
		}
	}
	.theme-preview[data-theme='warm-light'] {
		--pv-bg: #f4f0e9;
		--pv-surface: #fffdf8;
		--pv-line: rgb(26 37 32 / 0.12);
		--pv-accent: #d84b2a;
	}
	.theme-preview[data-theme='warm-dark'] {
		--pv-bg: #101512;
		--pv-surface: #17201a;
		--pv-line: rgb(255 255 255 / 0.08);
		--pv-accent: #f06a48;
	}
	.theme-preview[data-theme='cool-light'] {
		--pv-bg: #f5f6f8;
		--pv-surface: #ffffff;
		--pv-line: rgb(16 22 40 / 0.1);
		--pv-accent: #2a4dff;
	}
	.theme-preview[data-theme='cool-dark'] {
		--pv-bg: #0b0c0f;
		--pv-surface: #121419;
		--pv-line: rgb(255 255 255 / 0.08);
		--pv-accent: #7590ff;
	}
</style>
