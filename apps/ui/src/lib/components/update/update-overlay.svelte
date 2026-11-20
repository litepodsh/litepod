<script lang="ts">
	import { t } from '#lib/i18n/index.svelte.js';
	import { updateState } from '#lib/state/update.svelte.js';
	import { createArt, UPDATE_VARIANTS, type UpdateVariant } from './update-art';

	// Reduced motion freezes each animation on a frame where it reads well.
	const STILL: Record<UpdateVariant, number> = {
		assemble: 4900,
		version: 4100,
		apps: 5900,
		card: 4900,
	};
	const BAR = 12;

	let variant = $state<UpdateVariant>('assemble');
	let startedAt = 0;
	let elapsed = $state('00:00');
	let head = $state(0);
	let root = $state<HTMLElement>();
	let canvas = $state<HTMLCanvasElement>();
	let section = $state<HTMLElement>();

	const preparing = $derived(updateState.phase === 'preparing');
	const versions = $derived(updateState.versions);

	// A different look each time an update starts; pre so the first paint already uses it.
	$effect.pre(() => {
		if (!updateState.applying) return;
		variant = UPDATE_VARIANTS[Math.floor(Math.random() * UPDATE_VARIANTS.length)];
		startedAt = Date.now();
	});

	$effect(() => {
		if (!canvas || !root) return;
		const box = root;
		const card = variant === 'card';
		const still = matchMedia('(prefers-reduced-motion: reduce)').matches ? STILL[variant] : null;
		const art = createArt(canvas);

		function resize() {
			if (card || !section) return art.resize(canvas!.getBoundingClientRect().height);
			const play = Math.max(160, section.getBoundingClientRect().top - 24);
			box.style.setProperty('--lp-play', `${play}px`);
			art.resize(play);
		}
		resize();
		art.readColors(box);
		const observer = new ResizeObserver(resize);
		observer.observe(box);

		let raf = 0;
		let colorsAt = 0;
		function loop(now: number) {
			// Cheap way to follow a light/dark switch while the overlay is up.
			if (now - colorsAt > 500) {
				art.readColors(box);
				colorsAt = now;
			}
			art.draw(still ?? now, {
				variant,
				preparing,
				versions,
				labels: {
					current: t('common.update.current'),
					next: t('common.update.new'),
					apps: t('common.update.apps_online'),
					stages: [
						t('common.update.stage_panel'),
						t('common.update.stage_backup'),
						t('common.update.stage_image'),
						t('common.update.stage_restart'),
						t('common.update.stage_done'),
					],
				},
			});
			const secs = Math.floor((Date.now() - startedAt) / 1000);
			elapsed = `${String(Math.floor(secs / 60)).padStart(2, '0')}:${String(secs % 60).padStart(2, '0')}`;
			head = still === null ? Math.floor(now / 160) % (BAR + 4) : BAR;
			raf = requestAnimationFrame(loop);
		}
		raf = requestAnimationFrame(loop);

		return () => {
			cancelAnimationFrame(raf);
			observer.disconnect();
		};
	});
</script>

{#snippet content()}
	<p class="eyebrow mono">
		<span class="pulse"></span>
		<span>
			{t('common.update.eyebrow')}{#if versions}&nbsp;· v{versions.from} → v{versions.to}{/if}
		</span>
	</p>
	<h1>{preparing ? t('common.update.preparing') : t('common.update.waiting')}</h1>
	<p class="lede">
		{preparing ? t('common.update.preparing_hint') : t('common.update.waiting_hint')}
	</p>
	<div class="status mono" aria-hidden="true">
		<span>
			{preparing ? t('common.update.asking_server') : `${t('common.update.elapsed')} ${elapsed}`}
		</span>
		<span class="bar">
			{#each { length: BAR }, i (i)}
				<span class:on={i <= head && i > head - 4}></span>
			{/each}
		</span>
	</div>
	{#if variant === 'assemble' && !preparing}
		<ul class="steps mono">
			<li>{t('common.update.stage_backup')}</li>
			<li>{t('common.update.step_image')}</li>
			<li>{t('common.update.stage_restart')}</li>
			<li>{t('common.update.step_reload')}</li>
		</ul>
	{/if}
{/snippet}

{#snippet footer()}
	<footer class="mono">
		{#if versions}<p>litepod · v{versions.from}</p>{/if}
		<span class="brand">
			<svg viewBox="0 0 120 120" aria-hidden="true">
				<rect x="10" y="10" width="100" height="100" rx="26" fill="var(--lp-accent)" />
				<rect x="34" y="42" width="52" height="36" rx="18" fill="#fff" transform="rotate(-24 60 60)" />
			</svg>
			Litepod
		</span>
	</footer>
{/snippet}

{#if updateState.applying}
	<div
		bind:this={root}
		class="lp-update"
		class:center={variant === 'version'}
		class:overlay={variant === 'card'}
		role="alert"
		aria-live="polite">
		{#if variant === 'card'}
			<div class="card">
				<div class="art grid"><canvas bind:this={canvas} aria-hidden="true"></canvas></div>
				<div class="body">
					{@render content()}
					{@render footer()}
				</div>
			</div>
		{:else}
			<div class="board grid" aria-hidden="true"><canvas bind:this={canvas}></canvas></div>
			<div class="hud mono" aria-hidden="true">
				<p>{variant === 'apps' ? t('common.update.panel_restarting') : 'Litepod'}</p>
				{#if !preparing}<p>{t('common.update.elapsed')} <b>{elapsed}</b></p>{/if}
			</div>
			<main class="page">
				<section bind:this={section}>{@render content()}</section>
				{@render footer()}
			</main>
		{/if}
	</div>
{/if}

<style>
	/* Same look as the API's error and maintenance pages. */
	.lp-update {
		--lp-ground: #fafafa;
		--lp-cell: #efefef;
		--lp-ink: #0f0f0f;
		--lp-muted: #5e5e5e;
		--lp-accent: #d4410b;
		position: fixed;
		inset: 0;
		z-index: 100;
		overflow-y: auto;
		background: var(--lp-ground);
		color: var(--lp-ink);
		font-family: ui-sans-serif, system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif;
		-webkit-font-smoothing: antialiased;
	}
	:global(.dark) .lp-update {
		--lp-ground: #0a0a0a;
		--lp-cell: #1a1a1a;
		--lp-ink: #fff;
		--lp-muted: #b0b0b0;
		--lp-accent: #ff6a1f;
	}
	.lp-update.overlay {
		display: grid;
		place-items: center;
		padding: clamp(0.75rem, 3vw, 1.5rem);
		background: color-mix(in oklab, var(--lp-ground) 72%, transparent);
		backdrop-filter: blur(6px) saturate(0.7);
	}
	.mono {
		font: 500 0.75rem/1.5 ui-monospace, 'SF Mono', Menlo, monospace;
	}
	.grid::before {
		content: '';
		position: absolute;
		inset: 0;
		background-color: var(--lp-cell);
		background-image:
			linear-gradient(to right, var(--lp-ground) 1px, transparent 1px),
			linear-gradient(to bottom, var(--lp-ground) 1px, transparent 1px);
		background-size: 10px 10px;
	}
	.board {
		position: fixed;
		inset: 0;
	}
	.board::before {
		mask-image: linear-gradient(#000 var(--lp-play, 58vh), transparent calc(var(--lp-play, 58vh) + 8rem));
	}
	canvas {
		position: relative;
		display: block;
		width: 100%;
		height: 100%;
	}
	.hud {
		position: fixed;
		inset: 0 0 auto 0;
		display: flex;
		justify-content: space-between;
		gap: 1rem;
		padding: 1.1rem clamp(1rem, 3vw, 1.5rem);
		color: var(--lp-muted);
		pointer-events: none;
	}
	.hud p {
		margin: 0;
	}
	.hud b {
		color: var(--lp-ink);
		font-weight: 500;
		font-variant-numeric: tabular-nums;
	}
	.page {
		position: relative;
		min-height: 100dvh;
		display: flex;
		flex-direction: column;
		justify-content: flex-end;
		max-width: 72rem;
		margin: 0 auto;
		/* Keeps room for the art on short screens (landscape phones); the text scrolls. */
		padding: 11.5rem clamp(1.25rem, 5vw, 3rem) clamp(1.25rem, 4vh, 2.5rem);
	}
	.center .page {
		max-width: 44rem;
		align-items: center;
		text-align: center;
	}
	.eyebrow {
		display: inline-flex;
		align-items: center;
		gap: 0.6rem;
		margin: 0 0 1rem;
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: var(--lp-muted);
	}
	.pulse {
		flex: none;
		width: 0.5rem;
		height: 0.5rem;
		background: var(--lp-accent);
		animation: lp-pulse 1.6s steps(2, jump-none) infinite;
	}
	@keyframes lp-pulse {
		50% {
			opacity: 0.25;
		}
	}
	h1 {
		margin: 0;
		max-width: 18ch;
		font-size: clamp(1.75rem, 6vw, 3.5rem);
		line-height: 1;
		font-weight: 600;
		letter-spacing: -0.045em;
		text-wrap: balance;
	}
	.center h1 {
		margin-inline: auto;
	}
	.lede {
		margin: 1.1rem 0 0;
		max-width: 36rem;
		font-size: clamp(0.95rem, 1.5vw, 1.0625rem);
		line-height: 1.55;
		color: var(--lp-muted);
	}
	.center .lede {
		margin-inline: auto;
	}
	.status {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 0.75rem;
		margin-top: 1.5rem;
		color: var(--lp-muted);
	}
	.center .status {
		justify-content: center;
	}
	.bar {
		display: inline-flex;
		gap: 2px;
	}
	.bar span {
		width: 8px;
		height: 8px;
		background: color-mix(in oklab, var(--lp-ink) 18%, transparent);
	}
	.bar span.on {
		background: var(--lp-accent);
	}
	.steps {
		display: flex;
		flex-wrap: wrap;
		gap: 0.4rem 1.25rem;
		margin: 1.25rem 0 0;
		padding: 0;
		list-style: none;
		color: var(--lp-muted);
	}
	.steps li {
		display: inline-flex;
		align-items: center;
		gap: 0.5rem;
	}
	.steps li::before {
		content: '';
		width: 6px;
		height: 6px;
		background: color-mix(in oklab, var(--lp-ink) 25%, transparent);
	}
	footer {
		display: flex;
		flex-wrap: wrap;
		justify-content: space-between;
		gap: 0.5rem 1rem;
		width: 100%;
		margin-top: 1.75rem;
		padding-top: 1rem;
		border-top: 1px solid color-mix(in oklab, var(--lp-ink) 10%, transparent);
		color: var(--lp-muted);
	}
	.center footer {
		flex-direction: column;
		align-items: center;
	}
	footer p {
		margin: 0;
	}
	.brand {
		display: inline-flex;
		align-items: center;
		gap: 0.5rem;
	}
	.brand svg {
		width: 1rem;
		height: 1rem;
	}

	.card {
		width: min(34rem, 100%);
		max-height: calc(100dvh - 1.5rem);
		overflow-y: auto;
		border: 1px solid color-mix(in oklab, var(--lp-ink) 14%, transparent);
		border-radius: 0.75rem;
		background: var(--lp-ground);
		box-shadow: 0 30px 80px -30px rgb(0 0 0 / 0.6);
	}
	.art {
		position: relative;
		height: clamp(7rem, 22vh, 11rem);
	}
	.art::after {
		content: '';
		position: absolute;
		inset: auto 0 0 0;
		height: 4rem;
		background: linear-gradient(transparent, var(--lp-ground));
	}
	.art canvas {
		position: absolute;
		inset: 0;
	}
	.body {
		padding: 0 clamp(1.1rem, 4vw, 1.75rem) clamp(1.1rem, 4vw, 1.5rem);
	}
	.card h1 {
		font-size: clamp(1.5rem, 5vw, 1.9rem);
	}
	.card .lede {
		font-size: 0.95rem;
	}

	@media (prefers-reduced-motion: reduce) {
		.pulse {
			animation: none;
		}
	}
</style>
