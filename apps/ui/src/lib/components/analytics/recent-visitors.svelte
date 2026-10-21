<script lang="ts">
	import BotIcon from '@lucide/svelte/icons/bot';
	import CheckIcon from '@lucide/svelte/icons/check';
	import CopyIcon from '@lucide/svelte/icons/copy';
	import ExternalLinkIcon from '@lucide/svelte/icons/external-link';
	import MonitorIcon from '@lucide/svelte/icons/monitor';
	import SearchIcon from '@lucide/svelte/icons/search';
	import SmartphoneIcon from '@lucide/svelte/icons/smartphone';
	import SquareTerminalIcon from '@lucide/svelte/icons/square-terminal';
	import XIcon from '@lucide/svelte/icons/x';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Switch } from '#lib/components/ui/switch/index.js';
	import { compactNumber, type VisitorEntry } from '#lib/clients/analytics-client.js';
	import { formatBytes } from '#lib/clients/container-metrics-client.js';
	import type { OrganizationApp } from '#lib/utils/organization-apps.js';
	import { countryName, flag } from '#lib/utils/countries.js';
	import { relativeTime } from '#lib/utils/relative-time.js';
	import { toast } from '#lib/utils/toast.js';
	import { i18n, t } from '#lib/i18n/index.svelte.js';

	/** The latest client IPs: who they are, where from, what with, and how much. */
	let {
		visitors,
		apps,
		country = $bindable(null),
	}: {
		visitors: VisitorEntry[];
		/** Organization view: adds an App column. */
		apps?: OrganizationApp[];
		/** Alpha-2 picked on the map. */
		country?: string | null;
	} = $props();

	let query = $state('');
	let peopleOnly = $state(false);

	const BROWSERS: Record<string, string> = {
		chrome: 'Chrome',
		safari: 'Safari',
		firefox: 'Firefox',
		edge: 'Edge',
		opera: 'Opera',
	};
	const SYSTEMS: Record<string, string> = {
		ios: 'iOS',
		android: 'Android',
		windows: 'Windows',
		macos: 'macOS',
		linux: 'Linux',
		chromeos: 'ChromeOS',
	};
	const automated = (visitor: VisitorEntry) => visitor.client === 'bot' || visitor.client === 'tool';
	function clientLabel(key: string): string {
		if (BROWSERS[key]) return BROWSERS[key];
		if (key === 'bot' || key === 'tool' || key === 'unknown') return t(`analytics.client.${key}`);
		return t('analytics.client.other');
	}
	const systemLabel = (key: string) => SYSTEMS[key] ?? '';

	let appById = $derived(new Map((apps ?? []).map((app) => [app.application.id, app])));
	const place = (visitor: VisitorEntry) =>
		[visitor.city, visitor.region, visitor.country && countryName(visitor.country, i18n.locale)]
			.filter(Boolean)
			.join(', ');

	let rows = $derived.by(() => {
		const needle = query.trim().toLowerCase();
		return visitors.filter(
			(visitor) =>
				(!country || visitor.country === country) &&
				(!peopleOnly || !automated(visitor)) &&
				(!needle ||
					visitor.ip.toLowerCase().includes(needle) ||
					place(visitor).toLowerCase().includes(needle) ||
					clientLabel(visitor.client).toLowerCase().includes(needle)),
		);
	});
	let automatedCount = $derived(visitors.filter(automated).length);

	// Re-render relative times every 30s without refetching.
	let now = $state(Date.now());
	$effect(() => {
		const timer = setInterval(() => (now = Date.now()), 30_000);
		return () => clearInterval(timer);
	});
	const ago = (ms: number) => relativeTime(ms, now, i18n.locale);
	const at = (ms: number) =>
		new Date(ms).toLocaleString(i18n.locale, { dateStyle: 'medium', timeStyle: 'short' });
	const count = (value: number) => compactNumber(value, i18n.locale);

	let copied = $state<string | null>(null);
	async function copy(ip: string) {
		try {
			await navigator.clipboard.writeText(ip);
			copied = ip;
			setTimeout(() => {
				if (copied === ip) copied = null;
			}, 1500);
		} catch {
			toast.error(t('analytics.visitors.copy_failed'));
		}
	}
</script>

<section class="lp-raised flex min-w-0 flex-col rounded-2xl border bg-card">
	<header class="lp-groove-b flex flex-col gap-3 px-5 py-3.5 sm:flex-row sm:items-center sm:justify-between">
		<div class="min-w-0">
			<h2 class="text-[15px] font-semibold tracking-tight">{t('analytics.visitors.title')}</h2>
			<p class="mt-0.5 text-xs text-muted-foreground">
				{t('analytics.visitors.subtitle', { count: visitors.length })}
			</p>
		</div>
		<div class="flex flex-wrap items-center gap-x-4 gap-y-2">
			<label class="relative block w-full sm:w-56">
				<span class="sr-only">{t('analytics.visitors.search')}</span>
				<SearchIcon
					class="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" />
				<Input
					bind:value={query}
					placeholder={t('analytics.visitors.search')}
					class="h-8 pl-8 text-sm" />
			</label>
			<label class="flex items-center gap-2 text-sm text-muted-foreground">
				<Switch bind:checked={peopleOnly} />
				{t('analytics.visitors.people_only')}
				{#if automatedCount}
					<span class="text-xs tabular-nums">({automatedCount})</span>
				{/if}
			</label>
		</div>
	</header>

	{#if country}
		<div class="flex items-center gap-2 px-5 pt-3 text-xs text-muted-foreground">
			{t('analytics.visitors.filtered_by')}
			<button
				type="button"
				class="flex items-center gap-1.5 rounded-full border bg-muted px-2.5 py-0.5 text-foreground hover:border-foreground/25"
				onclick={() => (country = null)}
				aria-label={t('analytics.geo.clear_filter')}>
				<span aria-hidden="true">{flag(country)}</span>
				{countryName(country, i18n.locale)}
				<XIcon class="size-3 text-muted-foreground" />
			</button>
		</div>
	{/if}

	{#if rows.length === 0}
		<p class="px-5 py-8 text-center text-sm text-muted-foreground">
			{visitors.length ? t('analytics.visitors.no_match') : t('analytics.panel.no_rows')}
		</p>
	{:else}
		<div class="max-h-[34rem] overflow-auto">
			<table class="w-full min-w-[720px] text-sm">
				<thead class="sticky top-0 z-[1] bg-card text-left text-xs text-muted-foreground">
					<tr class="[&>th]:px-3 [&>th]:py-2.5 [&>th]:font-medium [&>th:first-child]:pl-5 [&>th:last-child]:pr-5">
						<th>{t('analytics.visitors.ip')}</th>
						<th>{t('analytics.visitors.location')}</th>
						<th>{t('analytics.visitors.client')}</th>
						{#if apps}<th>{t('analytics.dashboard.app')}</th>{/if}
						<th class="text-right">{t('analytics.visitors.requests')}</th>
						<th class="text-right">{t('analytics.visitors.last_seen')}</th>
					</tr>
				</thead>
				<tbody>
					{#each rows as visitor (`${visitor.ip}:${visitor.application_id ?? ''}`)}
						{@const bot = automated(visitor)}
						{@const app = visitor.application_id ? appById.get(visitor.application_id) : undefined}
						<tr
							class="group border-t border-foreground/[0.06] transition-colors hover:bg-muted/40 [&>td]:px-3 [&>td]:py-2.5 [&>td:first-child]:pl-5 [&>td:last-child]:pr-5">
							<td class="whitespace-nowrap">
								<span class="flex items-center gap-1">
									<span class="font-mono text-[13px]">{visitor.ip}</span>
									<button
										type="button"
										class="grid size-6 place-items-center rounded-md text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100 hover:bg-muted hover:text-foreground focus-visible:opacity-100"
										onclick={() => copy(visitor.ip)}
										aria-label={t('analytics.visitors.copy_ip')}>
										{#if copied === visitor.ip}
											<CheckIcon class="size-3.5" />
										{:else}
											<CopyIcon class="size-3.5" />
										{/if}
									</button>
									<a
										href="https://ipinfo.io/{encodeURIComponent(visitor.ip)}"
										target="_blank"
										rel="noopener noreferrer"
										class="grid size-6 place-items-center rounded-md text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100 hover:bg-muted hover:text-foreground focus-visible:opacity-100"
										aria-label={t('analytics.visitors.lookup_ip')}
										title={t('analytics.visitors.lookup_ip')}>
										<ExternalLinkIcon class="size-3.5" />
									</a>
								</span>
							</td>
							<td class="max-w-64">
								{#if visitor.country}
									<span class="flex min-w-0 items-center gap-2">
										<span class="text-base leading-none" aria-hidden="true">{flag(visitor.country)}</span>
										<span class="min-w-0">
											<span class="block truncate">
												{visitor.city ?? countryName(visitor.country, i18n.locale)}
											</span>
											{#if visitor.city}
												<span class="block truncate text-xs text-muted-foreground">
													{[visitor.region, countryName(visitor.country, i18n.locale)]
														.filter(Boolean)
														.join(', ')}
												</span>
											{/if}
										</span>
									</span>
								{:else}
									<span class="text-muted-foreground">{t('analytics.visitors.unknown_location')}</span>
								{/if}
							</td>
							<td class="whitespace-nowrap">
								<span class="flex items-center gap-2">
									<span
										class="grid size-7 shrink-0 place-items-center rounded-lg {bot
											? 'bg-[color-mix(in_oklch,var(--lp-info)_14%,transparent)] text-[var(--lp-info)]'
											: 'bg-muted text-muted-foreground'}">
										{#if visitor.client === 'bot'}
											<BotIcon class="size-3.5" />
										{:else if visitor.client === 'tool'}
											<SquareTerminalIcon class="size-3.5" />
										{:else if visitor.os === 'ios' || visitor.os === 'android'}
											<SmartphoneIcon class="size-3.5" />
										{:else}
											<MonitorIcon class="size-3.5" />
										{/if}
									</span>
									<span class="min-w-0">
										<span class="block">{clientLabel(visitor.client)}</span>
										{#if systemLabel(visitor.os)}
											<span class="block text-xs text-muted-foreground">{systemLabel(visitor.os)}</span>
										{/if}
									</span>
								</span>
							</td>
							{#if apps}
								<td class="max-w-40 truncate">
									{#if app}
										<a href={app.href} class="underline-offset-4 hover:underline">{app.application.name}</a>
									{:else}
										<span class="text-muted-foreground">{t('analytics.panel.other_app')}</span>
									{/if}
								</td>
							{/if}
							<td class="text-right whitespace-nowrap tabular-nums">
								<span class="block">{count(visitor.requests)}</span>
								<span class="block text-xs text-muted-foreground">
									{#if visitor.errors}
										<span class="text-destructive"
											>{t('analytics.panel.errors_count', { count: count(visitor.errors) })}</span>
									{:else}
										{formatBytes(visitor.bytes_out)}
									{/if}
								</span>
							</td>
							<td
								class="text-right whitespace-nowrap tabular-nums"
								title={t('analytics.visitors.seen_between', {
									first: at(visitor.first_seen),
									last: at(visitor.last_seen),
								})}>
								<span class="block">{ago(visitor.last_seen)}</span>
								<span class="block text-xs text-muted-foreground">
									{t('analytics.visitors.first_seen', { when: ago(visitor.first_seen) })}
								</span>
							</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</div>
	{/if}
</section>
