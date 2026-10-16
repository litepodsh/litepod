<script lang="ts">
	import * as Empty from '#lib/components/ui/empty/index.js';
	import ScrollTextIcon from '@lucide/svelte/icons/scroll-text';
	import SkeletonRows from '#lib/components/custom/skeleton-rows.svelte';
	import { onMount } from 'svelte';
	import * as Table from '#lib/components/ui/table/index.js';
	import * as Avatar from '#lib/components/ui/avatar/index.js';
	import * as Select from '#lib/components/ui/select/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import SoftSwitch from '#lib/components/custom/soft-switch.svelte';
	import { Label } from '#lib/components/ui/label/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import DownloadIcon from '@lucide/svelte/icons/download';
	import XIcon from '@lucide/svelte/icons/x';
	import { avatarUrl, initials } from '#lib/utils/avatar.js';
	import {
		listAuditLogs,
		exportAuditLogsUrl,
		formatAuditAction,
		AUDIT_ACTION_LABELS,
		type AuditLogDto,
		type AuditLogFilters,
	} from '#lib/clients/audit-log-client.js';
	import AuditLogDetail from './audit-log-detail.svelte';
	import { updateOrganization, type OrganizationDto } from '#lib/clients/organization-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		organizationId,
		organization = $bindable<OrganizationDto | null>(null),
	}: {
		organizationId: string;
		organization?: OrganizationDto | null;
	} = $props();

	const PAGE_SIZE = 50;
	const ACTION_OPTIONS = Object.entries(AUDIT_ACTION_LABELS).map(([value, label]) => ({
		value,
		label,
	}));

	let logs = $state<AuditLogDto[]>([]);
	let loading = $state(true);
	let loadingMore = $state(false);
	let hasMore = $state(true);
	let togglingAudit = $state(false);
	let retentionDays = $state(90);
	let savingRetention = $state(false);
	let seededFor = $state<string | null>(null);
	let exportRange = $state<'7' | '30' | '365' | 'custom'>('7');
	let exportFrom = $state('');
	let exportTo = $state('');
	let selected = $state<AuditLogDto | null>(null);
	let detailOpen = $state(false);

	// Filter state. `null` / empty string = "no filter on this dimension".
	// All fields are reactive so the `$effect` below refetches on change.
	let filterAction = $state<string>('');
	let filterTargetType = $state<string>('');
	let filterOutcome = $state<string>('');
	let filterFromDate = $state<string>('');
	let filterToDate = $state<string>('');
	let filterTargetId = $state<string>('');
	let filterActor = $state<string>('');

	// Reseed the retention input from the loaded org once, not on every
	// `organization` write this panel itself makes (the toggle/save below).
	$effect(() => {
		if (organization && seededFor !== organization.id) {
			retentionDays = organization.audit_logs_retention_days;
			seededFor = organization.id;
		}
	});

	function formatDate(value: string) {
		return new Date(value.endsWith('Z') ? value : `${value}Z`).toLocaleString();
	}

	function buildFilters(): AuditLogFilters | undefined {
		const filters: AuditLogFilters = {};
		if (filterAction) filters.action = filterAction;
		if (filterTargetType) filters.target_type = filterTargetType;
		if (filterOutcome) filters.outcome = filterOutcome as 'success' | 'failure';
		if (filterTargetId.trim()) filters.target_id = filterTargetId.trim();
		if (filterActor.trim()) filters.actor_user_id = filterActor.trim();
		if (filterFromDate) filters.from = `${filterFromDate} 00:00:00`;
		if (filterToDate) filters.to = `${filterToDate} 23:59:59`;
		return Object.keys(filters).length > 0 ? filters : undefined;
	}

	function clearFilters() {
		filterAction = '';
		filterTargetType = '';
		filterOutcome = '';
		filterFromDate = '';
		filterToDate = '';
		filterTargetId = '';
		filterActor = '';
	}

	const hasActiveFilters = $derived(
		!!(
			filterAction ||
			filterTargetType ||
			filterOutcome ||
			filterFromDate ||
			filterToDate ||
			filterTargetId.trim() ||
			filterActor.trim()
		),
	);

	async function loadFirstPage() {
		loading = true;
		try {
			logs = await listAuditLogs(organizationId, {
				limit: PAGE_SIZE,
				filters: buildFilters(),
			});
			hasMore = logs.length === PAGE_SIZE;
		} catch (err) {
			toast.error(err instanceof Error ? err.message : t('organizations.audit_logs.load_failed'));
		} finally {
			loading = false;
		}
	}

	// Refetch when any filter changes — debounce isn't worth it; the API is
	// fast and the panel re-mounts after a toggle to be obviously responsive.
	$effect(() => {
		// Touch every reactive to register them
		void filterAction;
		void filterTargetType;
		void filterOutcome;
		void filterFromDate;
		void filterToDate;
		void filterTargetId;
		void filterActor;
		if (seededFor) loadFirstPage();
	});

	async function loadMore() {
		const before = logs.at(-1)?.id;
		if (!before) return;
		loadingMore = true;
		try {
			const next = await listAuditLogs(organizationId, {
				limit: PAGE_SIZE,
				before,
				filters: buildFilters(),
			});
			logs = [...logs, ...next];
			hasMore = next.length === PAGE_SIZE;
		} catch (err) {
			toast.error(
				err instanceof Error ? err.message : t('organizations.audit_logs.load_more_failed'),
			);
		} finally {
			loadingMore = false;
		}
	}

	function selectEntry(entry: AuditLogDto) {
		selected = entry;
		detailOpen = true;
	}

	async function toggleEnabled(checked: boolean) {
		if (!organization) return;
		const previous = organization.audit_logs_enabled;
		organization.audit_logs_enabled = checked;
		togglingAudit = true;
		try {
			const updated = await updateOrganization(organizationId, { audit_logs_enabled: checked });
			organization.audit_logs_enabled = updated.audit_logs_enabled;
		} catch (err) {
			organization.audit_logs_enabled = previous;
			toast.error(err instanceof Error ? err.message : t('organizations.audit_logs.update_failed'));
		} finally {
			togglingAudit = false;
		}
	}

	async function saveRetention() {
		if (!organization) return;
		const days = Math.min(365, Math.max(1, Math.round(retentionDays) || 1));
		retentionDays = days;
		if (days === organization.audit_logs_retention_days) return;
		savingRetention = true;
		try {
			const updated = await updateOrganization(organizationId, {
				audit_logs_retention_days: days,
			});
			organization.audit_logs_retention_days = updated.audit_logs_retention_days;
			toast.success(t('organizations.audit_logs.retention_updated'));
		} catch (err) {
			retentionDays = organization.audit_logs_retention_days;
			toast.error(
				err instanceof Error ? err.message : t('organizations.audit_logs.retention_failed'),
			);
		} finally {
			savingRetention = false;
		}
	}

	/** SQLite's own `datetime()` format (`YYYY-MM-DD HH:MM:SS`, UTC) — matches
	 *  how `audit_logs.created_at` is stored, so the API can compare as plain
	 *  strings with no parsing on its side. */
	function formatSqliteDate(date: Date): string {
		return date.toISOString().slice(0, 19).replace('T', ' ');
	}

	function exportWindow(): { from?: string; to?: string } {
		if (exportRange === 'custom') {
			return {
				from: exportFrom ? `${exportFrom} 00:00:00` : undefined,
				to: exportTo ? `${exportTo} 23:59:59` : undefined,
			};
		}
		const days = Number(exportRange);
		return { from: formatSqliteDate(new Date(Date.now() - days * 24 * 60 * 60 * 1000)) };
	}

	const EXPORT_RANGE_LABELS: Record<typeof exportRange, string> = {
		'7': 'organizations.audit_logs.range_7',
		'30': 'organizations.audit_logs.range_30',
		'365': 'organizations.audit_logs.range_365',
		custom: 'organizations.audit_logs.range_custom',
	};

	function exportCsv() {
		const a = document.createElement('a');
		a.href = exportAuditLogsUrl(organizationId, exportWindow());
		a.download = `audit-logs-${organizationId.slice(0, 8)}.csv`;
		document.body.appendChild(a);
		a.click();
		document.body.removeChild(a);
	}

	onMount(loadFirstPage);
</script>

<div class="flex flex-col gap-4">
	<div
		class="flex flex-col gap-3 lp-raised rounded-2xl border bg-card p-4 sm:flex-row sm:items-center sm:justify-between sm:gap-4">
		<div>
			<Label for="audit-logs-enabled">{t('organizations.audit_logs.title')}</Label>
			<p class="mt-1 text-sm text-muted-foreground">
				{t('organizations.audit_logs.description')}
			</p>
		</div>
		<SoftSwitch
			id="audit-logs-enabled"
			checked={organization?.audit_logs_enabled ?? false}
			disabled={togglingAudit || !organization}
			onCheckedChange={toggleEnabled} />
	</div>

	<div
		class="flex flex-col gap-3 lp-raised rounded-2xl border bg-card p-4 sm:flex-row sm:items-center sm:justify-between sm:gap-4">
		<div>
			<Label for="audit-logs-retention">{t('organizations.audit_logs.retention')}</Label>
			<p class="mt-1 text-sm text-muted-foreground">
				{t('organizations.audit_logs.retention_description')}
			</p>
		</div>
		<div class="flex items-center gap-2">
			<Input
				id="audit-logs-retention"
				type="number"
				min="1"
				max="365"
				class="w-20"
				disabled={savingRetention || !organization}
				bind:value={retentionDays}
				onchange={saveRetention} />
			<span class="text-muted-foreground text-sm">{t('organizations.audit_logs.days')}</span>
		</div>
	</div>

	<div class="flex flex-col gap-3 lp-raised rounded-2xl border bg-card p-4">
		<div class="flex items-center justify-between">
			<h3 class="text-sm font-medium">{t('organizations.audit_logs.filters')}</h3>
			{#if hasActiveFilters}
				<Button variant="ghost" size="sm" onclick={clearFilters}>
					<XIcon class="mr-1 size-3.5" />
					{t('organizations.common.clear')}
				</Button>
			{/if}
		</div>
		<div class="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
			<div class="grid gap-1.5">
				<Label for="audit-logs-filter-action">{t('organizations.audit_logs.action')}</Label>
				<Select.Root
					type="single"
					value={filterAction || undefined}
					onValueChange={(v) => (filterAction = v ?? '')}>
					<Select.Trigger id="audit-logs-filter-action" class="w-full">
						{ACTION_OPTIONS.find((o) => o.value === filterAction)?.label ??
							t('organizations.audit_logs.any_action')}
					</Select.Trigger>
					<Select.Content>
						<Select.Item value="">{t('organizations.audit_logs.any_action')}</Select.Item>
						{#each ACTION_OPTIONS as opt (opt.value)}
							<Select.Item value={opt.value}>{opt.label}</Select.Item>
						{/each}
					</Select.Content>
				</Select.Root>
			</div>
			<div class="grid gap-1.5">
				<Label for="audit-logs-filter-target-type"
					>{t('organizations.audit_logs.target_type')}</Label>
				<Input
					id="audit-logs-filter-target-type"
					placeholder={t('organizations.audit_logs.target_type_placeholder')}
					bind:value={filterTargetType} />
			</div>
			<div class="grid gap-1.5">
				<Label for="audit-logs-filter-outcome">{t('organizations.audit_logs.outcome')}</Label>
				<Select.Root
					type="single"
					value={filterOutcome || undefined}
					onValueChange={(v) => (filterOutcome = v ?? '')}>
					<Select.Trigger id="audit-logs-filter-outcome" class="w-full">
						{filterOutcome === 'success'
							? t('organizations.audit_logs.success')
							: filterOutcome === 'failure'
								? t('organizations.audit_logs.failure')
								: t('organizations.audit_logs.any_outcome')}
					</Select.Trigger>
					<Select.Content>
						<Select.Item value="">{t('organizations.audit_logs.any_outcome')}</Select.Item>
						<Select.Item value="success">{t('organizations.audit_logs.success')}</Select.Item>
						<Select.Item value="failure">{t('organizations.audit_logs.failure')}</Select.Item>
					</Select.Content>
				</Select.Root>
			</div>
			<div class="grid gap-1.5">
				<Label for="audit-logs-filter-from">{t('organizations.audit_logs.from')}</Label>
				<Input id="audit-logs-filter-from" type="date" bind:value={filterFromDate} />
			</div>
			<div class="grid gap-1.5">
				<Label for="audit-logs-filter-to">{t('organizations.audit_logs.to')}</Label>
				<Input id="audit-logs-filter-to" type="date" bind:value={filterToDate} />
			</div>
			<div class="grid gap-1.5">
				<Label for="audit-logs-filter-target-id">{t('organizations.audit_logs.target_id')}</Label>
				<Input
					id="audit-logs-filter-target-id"
					placeholder={t('organizations.audit_logs.target_id_placeholder')}
					bind:value={filterTargetId} />
			</div>
		</div>
	</div>

	<div
		class="flex flex-col gap-3 lp-raised rounded-2xl border bg-card p-4 sm:flex-row sm:items-end sm:justify-between">
		<div class="grid gap-1.5">
			<Label for="audit-logs-export-range">{t('organizations.audit_logs.export_range')}</Label>
			<Select.Root
				type="single"
				value={exportRange}
				onValueChange={(v) => (exportRange = v as typeof exportRange)}>
				<Select.Trigger id="audit-logs-export-range" class="w-40">
					{t(EXPORT_RANGE_LABELS[exportRange])}
				</Select.Trigger>
				<Select.Content>
					<Select.Item value="7">{t('organizations.audit_logs.range_7')}</Select.Item>
					<Select.Item value="30">{t('organizations.audit_logs.range_30')}</Select.Item>
					<Select.Item value="365">{t('organizations.audit_logs.range_365')}</Select.Item>
					<Select.Item value="custom">{t('organizations.audit_logs.range_custom')}</Select.Item>
				</Select.Content>
			</Select.Root>
		</div>
		{#if exportRange === 'custom'}
			<div class="grid gap-1.5">
				<Label for="audit-logs-export-from">{t('organizations.audit_logs.from')}</Label>
				<Input id="audit-logs-export-from" type="date" bind:value={exportFrom} />
			</div>
			<div class="grid gap-1.5">
				<Label for="audit-logs-export-to">{t('organizations.audit_logs.to')}</Label>
				<Input id="audit-logs-export-to" type="date" bind:value={exportTo} />
			</div>
		{/if}
		<Button
			variant="outline"
			size="sm"
			onclick={exportCsv}
			disabled={exportRange === 'custom' && !exportFrom && !exportTo}>
			<DownloadIcon class="size-4" />
			{t('organizations.audit_logs.export_csv')}
		</Button>
	</div>

	{#snippet actor(entry: AuditLogDto)}
		<Avatar.Root class="size-7 rounded-lg">
			{#if entry.actor_email}
				<Avatar.Image
					src={avatarUrl(entry.actor_name || entry.actor_email)}
					alt={t('organizations.common.avatar_alt', {
						name: entry.actor_name || entry.actor_email,
					})} />
			{/if}
			<Avatar.Fallback class="rounded-lg text-xs">
				{entry.actor_email ? initials(entry.actor_name || entry.actor_email) : '?'}
			</Avatar.Fallback>
		</Avatar.Root>
		<span class="min-w-0 truncate text-sm">
			{entry.actor_email ?? t('organizations.audit_logs.deleted_user')}
		</span>
	{/snippet}

	{#snippet outcomeBadge(entry: AuditLogDto)}
		{#if entry.outcome}
			<Badge variant={entry.outcome === 'success' ? 'secondary' : 'destructive'}>
				{entry.outcome}
			</Badge>
		{/if}
	{/snippet}

	{#snippet target(entry: AuditLogDto)}
		{#if entry.target_name}
			<span>{entry.target_name}</span>
			{#if entry.target_type}
				<span class="text-muted-foreground text-xs">({entry.target_type})</span>
			{/if}
		{:else if entry.target_type}
			<span class="text-muted-foreground text-xs">{entry.target_type}</span>
		{/if}
	{/snippet}

	{#if loading}
		<SkeletonRows rows={6} height="h-12" label={t('organizations.audit_logs.loading')} />
	{:else if logs.length === 0}
		<Empty.Root class="rounded-2xl border border-dashed border-foreground/15 bg-card">
			<Empty.Header>
				<Empty.Media variant="icon"><ScrollTextIcon /></Empty.Media>
				<Empty.Title>
					{hasActiveFilters
						? t('organizations.audit_logs.empty_filtered')
						: t('organizations.audit_logs.empty')}
				</Empty.Title>
				<Empty.Description>
					{#if hasActiveFilters}
						{t('organizations.audit_logs.empty_filtered_hint')}
					{:else}
						{t('organizations.audit_logs.empty_hint')}
					{/if}
				</Empty.Description>
			</Empty.Header>
		</Empty.Root>
	{:else}
		<div class="flex flex-col gap-3 sm:hidden">
			{#each logs as entry (entry.id)}
				<!-- svelte-ignore a11y_no_static_element_interactions -->
				<!-- svelte-ignore a11y_click_events_have_key_events -->
				<div
					role="button"
					tabindex="0"
					class="hover:bg-muted/40 flex cursor-pointer flex-col gap-2 lp-raised rounded-2xl border bg-card p-4 transition-colors"
					onclick={() => selectEntry(entry)}
					onkeydown={(e) => {
						if (e.key === 'Enter' || e.key === ' ') {
							e.preventDefault();
							selectEntry(entry);
						}
					}}>
					<div class="flex items-center gap-2">
						{@render actor(entry)}
						{@render outcomeBadge(entry)}
					</div>
					<p class="text-sm font-medium">{formatAuditAction(entry.action)}</p>
					{#if entry.target_name || entry.target_type}
						<p class="text-muted-foreground text-sm">{@render target(entry)}</p>
					{/if}
					<p class="text-muted-foreground text-xs">{formatDate(entry.created_at)}</p>
				</div>
			{/each}
		</div>
		<div class="lp-raised hidden overflow-x-auto rounded-2xl border bg-card sm:block">
			<Table.Root>
				<Table.Header>
					<Table.Row>
						<Table.Head>{t('organizations.audit_logs.when')}</Table.Head>
						<Table.Head>{t('organizations.audit_logs.who')}</Table.Head>
						<Table.Head>{t('organizations.audit_logs.action')}</Table.Head>
						<Table.Head>{t('organizations.audit_logs.target')}</Table.Head>
						<Table.Head class="w-20">{t('organizations.audit_logs.status')}</Table.Head>
						<Table.Head class="w-12"></Table.Head>
					</Table.Row>
				</Table.Header>
				<Table.Body>
					{#each logs as entry (entry.id)}
						<Table.Row class="hover:bg-muted/40 cursor-pointer" onclick={() => selectEntry(entry)}>
							<Table.Cell class="whitespace-nowrap text-sm">
								{formatDate(entry.created_at)}
							</Table.Cell>
							<Table.Cell>
								<div class="flex items-center gap-2">
									{@render actor(entry)}
								</div>
							</Table.Cell>
							<Table.Cell class="text-sm">{formatAuditAction(entry.action)}</Table.Cell>
							<Table.Cell class="text-sm">
								{@render target(entry)}
							</Table.Cell>
							<Table.Cell>
								{@render outcomeBadge(entry)}
							</Table.Cell>
							<Table.Cell>
								<Button
									size="sm"
									variant="ghost"
									onclick={(e) => {
										e.stopPropagation();
										selectEntry(entry);
									}}>
									{t('organizations.audit_logs.details')}
								</Button>
							</Table.Cell>
						</Table.Row>
					{/each}
				</Table.Body>
			</Table.Root>
		</div>
		{#if hasMore}
			<Button
				variant="outline"
				size="sm"
				disabled={loadingMore}
				onclick={loadMore}
				class="self-start">
				{loadingMore ? t('organizations.common.loading') : t('organizations.audit_logs.load_more')}
			</Button>
		{/if}
	{/if}
</div>

<AuditLogDetail bind:open={detailOpen} entry={selected} />
