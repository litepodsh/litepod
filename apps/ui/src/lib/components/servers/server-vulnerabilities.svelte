<script lang="ts">
	import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import {
		disableServerVulnerabilities,
		getServerVulnerabilities,
		scanServerVulnerabilities,
		type VulnerabilityScan,
	} from '#lib/clients/server-client.js';
	import { i18n, t } from '#lib/i18n/index.svelte.js';
	import { relativeTime } from '#lib/utils/relative-time.js';
	import { toast } from '#lib/utils/toast.js';

	let { organizationId, serverId }: { organizationId: string; serverId: string } = $props();

	const POLL_MS = 5000;
	const COUNTS = ['critical', 'high', 'medium', 'low'] as const;

	let scan = $state<VulnerabilityScan | null>(null);
	let loadError = $state('');
	let busy = $state(false);
	let search = $state('');

	const running = $derived(scan?.status === 'running');
	const findings = $derived.by(() => {
		const all = scan?.findings ?? [];
		const query = search.trim().toLowerCase();
		if (!query) return all;
		return all.filter(
			(finding) =>
				finding.id.toLowerCase().includes(query) || finding.package.toLowerCase().includes(query),
		);
	});

	async function load() {
		try {
			scan = await getServerVulnerabilities(organizationId, serverId);
			loadError = '';
		} catch (caught) {
			loadError = caught instanceof Error ? caught.message : t('servers.security.vuln_load_failed');
		}
	}

	async function startScan() {
		busy = true;
		try {
			scan = await scanServerVulnerabilities(organizationId, serverId);
		} catch (caught) {
			toast.error(caught instanceof Error ? caught.message : t('servers.security.vuln_failed'));
		} finally {
			busy = false;
		}
	}

	async function disable() {
		busy = true;
		try {
			const { disk_freed } = await disableServerVulnerabilities(organizationId, serverId);
			if (disk_freed) toast.success(t('servers.security.vuln_disabled_toast'));
			else toast.warning(t('servers.security.vuln_cleanup_failed'));
			await load();
		} catch (caught) {
			toast.error(caught instanceof Error ? caught.message : t('servers.security.vuln_failed'));
		} finally {
			busy = false;
		}
	}

	$effect(() => {
		void load();
	});

	$effect(() => {
		if (!running) return;
		const timer = setInterval(() => void load(), POLL_MS);
		return () => clearInterval(timer);
	});
</script>

<section class="space-y-3 rounded-lg border p-3">
	<div class="flex flex-wrap items-start justify-between gap-2">
		<div class="min-w-0 flex-1">
			<p class="text-sm font-medium">{t('servers.security.vuln_title')}</p>
			<p class="text-xs text-muted-foreground">{t('servers.security.vuln_description')}</p>
		</div>
		{#if scan?.enabled}
			<div class="flex gap-2">
				<Button size="sm" variant="ghost" disabled={busy || running} onclick={() => void disable()}>
					{t('servers.security.vuln_disable')}
				</Button>
				<Button
					size="sm"
					variant="outline"
					disabled={busy || running}
					onclick={() => void startScan()}>
					{#if running}<LoaderCircleIcon class="size-4 animate-spin" />{/if}
					{running ? t('servers.security.vuln_scanning') : t('servers.security.vuln_scan_now')}
				</Button>
			</div>
		{/if}
	</div>

	{#if loadError}
		<p class="text-xs text-destructive">{loadError}</p>
	{:else if scan && !scan.enabled}
		<p class="text-xs text-muted-foreground">{t('servers.security.vuln_requirements')}</p>
		<Button size="sm" disabled={busy} onclick={() => void startScan()}>
			{#if busy}<LoaderCircleIcon class="size-4 animate-spin" />{/if}
			{t('servers.security.vuln_enable')}
		</Button>
	{:else if scan}
		{#if running && !scan.summary}
			<p class="text-xs text-muted-foreground">{t('servers.security.vuln_first_scan')}</p>
		{/if}
		{#if scan.error && !running}
			<p class="flex items-start gap-1.5 text-xs text-warning">
				<TriangleAlertIcon class="mt-0.5 size-3.5 shrink-0" />
				<span class="min-w-0 break-words">{scan.error}</span>
			</p>
		{/if}
		{#if scan.summary}
			<div class="flex flex-wrap items-center gap-1.5">
				{#if scan.summary.known_exploited}
					<Badge variant="destructive">
						{t('servers.security.vuln_count_exploited', { count: scan.summary.known_exploited })}
					</Badge>
				{/if}
				{#each COUNTS as severity (severity)}
					{#if scan.summary[severity]}
						<Badge variant={severity === 'critical' ? 'destructive' : 'outline'}>
							{t(`servers.security.vuln_count_${severity}`, { count: scan.summary[severity] })}
						</Badge>
					{/if}
				{/each}
			</div>
			<p class="text-xs text-muted-foreground">
				{#if scan.scanned_at}
					{t('servers.security.vuln_last_scan', {
						when: relativeTime(scan.scanned_at, Date.now(), i18n.locale),
					})} ·
				{/if}
				{t('servers.security.vuln_fixable', { count: scan.summary.fixable })}
			</p>
			{#if scan.summary.total === 0}
				<p class="text-xs">{t('servers.security.vuln_none')}</p>
			{:else}
				<Input
					bind:value={search}
					placeholder={t('servers.security.vuln_search')}
					aria-label={t('servers.security.vuln_search')}
					class="h-8 text-xs" />
				<ul class="max-h-64 divide-y overflow-y-auto rounded-md border text-xs">
					{#each findings as finding (finding.id)}
						<li class="flex flex-wrap items-center gap-x-2 gap-y-1 p-2">
							{#if finding.url}
								<a
									href={finding.url}
									target="_blank"
									rel="noreferrer"
									class="font-mono font-medium underline-offset-2 hover:underline">{finding.id}</a>
							{:else}
								<span class="font-mono font-medium">{finding.id}</span>
							{/if}
							<Badge variant={finding.severity === 'critical' ? 'destructive' : 'outline'}>
								{t(`servers.security.vuln_severity_${finding.severity}`)}
							</Badge>
							{#if finding.known_exploited}
								<Badge variant="destructive">{t('servers.security.vuln_exploited')}</Badge>
							{/if}
							<span class="min-w-0 break-all text-muted-foreground">
								{finding.package}
								{finding.version}
							</span>
							<span class="ml-auto text-muted-foreground">
								{finding.fixed_in
									? t('servers.security.vuln_fixed_in', { version: finding.fixed_in })
									: t('servers.security.vuln_no_fix')}
							</span>
						</li>
					{:else}
						<li class="p-2 text-muted-foreground">{t('servers.security.vuln_no_match')}</li>
					{/each}
				</ul>
				{#if scan.truncated}
					<p class="text-xs text-muted-foreground">{t('servers.security.vuln_truncated')}</p>
				{/if}
			{/if}
		{/if}
	{/if}
</section>
