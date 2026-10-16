<script lang="ts">
	import { page } from '$app/state';
	import { scopeHero } from '#lib/state/shell.svelte.js';
	import { cn } from '#lib/utils.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';

	/** The info card between the header and the tabs. Pages fill it with `setScopeHero`. */
	let hero = $derived(scopeHero.current?.path === page.url.pathname ? scopeHero.current : null);

	// Phones keep the hero to title and actions; stats and siblings fold away.
	let expanded = $state(false);

	let strip = $state<HTMLElement | null>(null);
	$effect(() => {
		void hero?.siblings;
		strip
			?.querySelector<HTMLElement>('[aria-current="page"]')
			?.scrollIntoView?.({ block: 'nearest', inline: 'center' });
	});
</script>

{#if hero}
	<section class="lp-groove-b bg-card/60">
		<div class="shell-col flex flex-col gap-4 px-4 pt-5 pb-4 sm:px-6 sm:pt-6">
			<div class="flex flex-wrap items-start justify-between gap-3">
				<div class="flex min-w-0 items-center gap-3.5">
					{#if hero.avatar}
						<span
							class="lp-raised flex size-11 shrink-0 overflow-hidden rounded-xl border sm:size-12"
							>{@render hero.avatar()}</span>
					{/if}
					<div class="min-w-0">
						<h1 class="flex flex-wrap items-center gap-2.5 text-xl font-semibold tracking-tight">
							<span class="truncate">{hero.title}</span>
							{#if hero.status}{@render hero.status()}{/if}
						</h1>
						{#if hero.meta}
							<div
								class="mt-1 flex min-w-0 flex-wrap items-center gap-2 text-sm text-muted-foreground">
								{@render hero.meta()}
							</div>
						{/if}
					</div>
				</div>
				{#if hero.actions}
					<div class="flex flex-wrap items-center gap-2 max-sm:w-full">
						{@render hero.actions()}
					</div>
				{/if}
			</div>

			{#if hero.stats?.length || hero.siblings?.length}
				<button
					type="button"
					class="flex items-center gap-1 self-start text-xs text-muted-foreground sm:hidden"
					aria-expanded={expanded}
					aria-controls="hero-details"
					onclick={() => (expanded = !expanded)}>
					{expanded ? t('common.common.hide_details') : t('common.common.show_details')}
					<ChevronDownIcon class={cn('size-3.5 transition-transform', expanded && 'rotate-180')} />
				</button>
			{/if}

			<div id="hero-details" class={cn('flex flex-col gap-4', !expanded && 'max-sm:hidden')}>
				{#if hero.stats?.length}
					<dl class="stats lp-raised rounded-2xl border bg-background" data-testid="hero-stats">
						{#each hero.stats as stat (stat.label)}
							<div class="stat">
								<dt>{stat.label}</dt>
								<dd>
									{#if stat.href}<a href={stat.href} class="hover:underline">{stat.value}</a>
									{:else}{stat.value}{/if}
								</dd>
							</div>
						{/each}
					</dl>
				{/if}

				{#if hero.siblings?.length}
					<nav bind:this={strip} class="siblings" aria-label={hero.siblingsLabel}>
						{#if hero.siblingsLabel}<span class="me-1 shrink-0 text-xs text-muted-foreground"
								>{hero.siblingsLabel}</span
							>{/if}
						{#each hero.siblings as sibling (sibling.id)}
							<a
								href={sibling.href}
								aria-current={sibling.current ? 'page' : undefined}
								class={cn('sib', sibling.tone && `sib-${sibling.tone}`)}>
								{#if sibling.tone}<span class="pip" aria-hidden="true"></span>{/if}
								{sibling.label}
							</a>
						{/each}
					</nav>
				{/if}
			</div>
		</div>
	</section>
{/if}

<style>
	.stats {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(140px, 1fr));
		margin: 0;
	}
	.stat {
		min-width: 0;
		padding: 12px 16px;
	}
	/* Engraved column separators: a dark hairline with a light one beside it. */
	.stat + .stat {
		border-left: 1px solid var(--lp-emboss);
		box-shadow: inset 1px 0 0 var(--lp-bevel);
	}
	dt {
		margin-bottom: 3px;
		color: var(--muted-foreground);
		font-size: 12px;
	}
	dd {
		overflow: hidden;
		font-weight: 500;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.siblings {
		display: flex;
		align-items: center;
		gap: 6px;
		padding-bottom: 2px;
		overflow-x: auto;
		scrollbar-width: none;
	}
	.sib {
		display: inline-flex;
		flex: none;
		align-items: center;
		gap: 6px;
		height: 28px;
		padding: 0 10px;
		border: 1px solid color-mix(in srgb, var(--foreground) 10%, transparent);
		border-radius: 999px;
		color: var(--muted-foreground);
		font-size: 12.5px;
		white-space: nowrap;
		transition:
			border-color 0.15s var(--lp-ease),
			color 0.15s var(--lp-ease);
	}
	.sib:hover {
		border-color: color-mix(in srgb, var(--foreground) 22%, transparent);
		color: var(--foreground);
	}
	.sib[aria-current='page'] {
		border-color: var(--foreground);
		background: var(--foreground);
		color: var(--background);
	}
	.pip {
		width: 6px;
		height: 6px;
		border-radius: 50%;
		background: var(--success);
	}
	.sib-warn .pip {
		background: var(--warning);
	}
	.sib-off .pip {
		background: var(--muted-foreground);
	}
	.sib[aria-current='page'] .pip {
		background: currentColor;
	}

	@media (max-width: 640px) {
		/* Stats become one swipeable row instead of a tall stack. */
		.stats {
			display: flex;
			gap: 0;
			overflow-x: auto;
			scroll-snap-type: x mandatory;
			scrollbar-width: none;
		}
		.stat {
			flex: none;
			min-width: 128px;
			padding: 10px 14px;
			scroll-snap-align: start;
		}
		.siblings {
			margin: 0 -16px;
			padding: 0 16px 2px;
		}
	}
</style>
