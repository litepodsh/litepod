<script lang="ts">
	import SkeletonRows from '#lib/components/custom/skeleton-rows.svelte';
	import { onMount, onDestroy } from 'svelte';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import ContainerIcon from '@lucide/svelte/icons/container';
	import DownloadIcon from '@lucide/svelte/icons/download';
	import CheckCircle2Icon from '@lucide/svelte/icons/check-circle-2';
	import AlertCircleIcon from '@lucide/svelte/icons/alert-circle';
	import ClockIcon from '@lucide/svelte/icons/clock';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Tooltip from '#lib/components/ui/tooltip/index.js';
	import { toast } from '#lib/utils/toast.js';
	import { toneClass, toneDot, type StatusTone } from '#lib/utils/status-tone.js';
	import { isInstallableUpdate, notifySystemUpdateChanged } from '#lib/utils/system-update.js';
	import { getSystemServiceStatuses } from '#lib/utils/system-service-status.js';
	import { formatDuration, formatRelative } from '#lib/utils/duration.js';
	import { updateState } from '#lib/state/update.svelte.js';
	import {
		getSystemVersion,
		triggerUpdateCheck,
		type SystemVersion,
	} from '#lib/clients/system-client.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let info = $state<SystemVersion | null>(null);
	let loading = $state(true);
	let loadError = $state<string | null>(null);
	let checking = $state(false);
	let pollTimer: ReturnType<typeof setInterval> | null = null;
	const channelLabel = $derived(info?.state.channel === 'alpha' ? 'Alpha' : 'Stable');
	const runningVersion = $derived(info?.version ?? '');
	const updateReady = $derived(isInstallableUpdate(info));
	const serviceStatuses = $derived(getSystemServiceStatuses(info));

	/** One pill for the updater: same precedence the badges always had. */
	const updateStatus = $derived.by((): { tone: StatusTone; label: string } => {
		if (!info || info.update_interval_secs === 0)
			return { tone: 'neutral', label: t('operations.system.disabled') };
		if (info.state.last_check_at && !info.state.last_check_succeeded)
			return { tone: 'danger', label: t('operations.system.check_failed') };
		if (updateReady) return { tone: 'warn', label: t('operations.system.update_available') };
		if (info.state.last_check_succeeded)
			return { tone: 'ok', label: t('operations.system.up_to_date') };
		return { tone: 'neutral', label: t('operations.system.not_checked') };
	});

	// Ticks every second so the "Last check" label counts up live.
	let now = $state(Date.now());
	let clockTimer: ReturnType<typeof setInterval> | null = null;
	const lastCheckLabel = $derived(formatRelative(info?.state.last_check_at ?? null, now));
	const checkIntervalLabel = $derived(
		info === null || info.update_interval_secs === 0
			? t('operations.system.disabled')
			: formatDuration(info.update_interval_secs),
	);

	// Uptimes are server-measured at load; advancing them by local elapsed time
	// keeps them ticking between polls without trusting the browser clock.
	let loadedAt = $state(Date.now());
	function uptimeLabel(secs: number | null) {
		return secs === null ? '—' : formatDuration(secs + (now - loadedAt) / 1_000);
	}

	async function load() {
		loadError = null;
		try {
			info = await getSystemVersion();
			loadedAt = Date.now();
		} catch (error) {
			info = null;
			loadError = error instanceof Error ? error.message : t('operations.system.load_failed');
			toast.error(loadError);
		} finally {
			loading = false;
		}
	}

	async function check() {
		if (checking) return;
		checking = true;
		try {
			const res = await triggerUpdateCheck();
			if (res.triggered) {
				toast.info(t('operations.system.check_scheduled'));
			} else {
				toast.warning(res.note);
			}
			setTimeout(async () => {
				await load();
				notifySystemUpdateChanged();
			}, 3000);
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('operations.system.check_failed_toast'),
			);
		} finally {
			checking = false;
		}
	}

	function startPolling() {
		pollTimer = setInterval(load, 60_000);
		clockTimer = setInterval(() => (now = Date.now()), 1_000);
	}

	// The System page checks sysadmin access before mounting this panel.
	onMount(async () => {
		await load();
		startPolling();
	});

	onDestroy(() => {
		if (pollTimer) clearInterval(pollTimer);
		if (clockTimer) clearInterval(clockTimer);
	});
</script>

{#snippet fact(label: string, value: string, mono = true)}
	<div class="lp-inset flex min-w-0 flex-col gap-0.5 rounded-lg bg-muted/60 px-3 py-2.5">
		<dt class="font-mono text-[11.5px] text-muted-foreground">{label}</dt>
		<dd class="text-[13px] font-medium break-all {mono ? 'font-mono' : ''}">{value}</dd>
	</div>
{/snippet}

{#snippet pill(tone: StatusTone, label: string)}
	<span
		class="inline-flex shrink-0 items-center gap-1.5 rounded-full px-2 py-0.5 text-xs font-medium {toneClass(
			tone,
		)}">
		<i class="size-1.5 rounded-full {toneDot(tone)}"></i>{label}
	</span>
{/snippet}

<div class="flex flex-col gap-4">
	<div>
		<h2 class="text-lg font-semibold tracking-tight">{t('menu.user_menu.system_info')}</h2>
		<p class="mt-1 text-sm text-muted-foreground">{t('operations.system.build_description')}</p>
	</div>

	{#if loading}
		<div class="lp-raised rounded-2xl border bg-card p-4">
			<SkeletonRows rows={3} height="h-10" label={t('operations.system.loading')} />
		</div>
	{:else if info}
		<!-- Build identity -->
		<section class="lp-raised min-w-0 overflow-hidden rounded-2xl border bg-card">
			<header class="lp-groove-b flex items-center gap-2.5 px-4 py-3">
				<span
					class="lp-inset grid size-[30px] shrink-0 place-items-center rounded-lg bg-muted text-muted-foreground">
					<ContainerIcon class="size-4" />
				</span>
				<h2 class="text-[15px] font-semibold tracking-tight">{t('operations.system.build')}</h2>
			</header>
			<dl class="grid gap-2.5 p-4 sm:grid-cols-2">
				{@render fact(t('operations.system.version'), info.version)}
				{@render fact(t('operations.system.git_sha'), info.git_sha)}
				{@render fact(t('operations.system.image'), info.image)}
				{@render fact(t('operations.system.container'), info.container_name)}
			</dl>
		</section>

		<!-- Updater status -->
		<section class="lp-raised min-w-0 overflow-hidden rounded-2xl border bg-card">
			<header
				class="lp-groove-b flex flex-col gap-3 px-4 py-3 sm:flex-row sm:items-start sm:justify-between">
				<div class="flex min-w-0 gap-2.5">
					<span
						class="lp-inset grid size-[30px] shrink-0 place-items-center rounded-lg bg-muted text-muted-foreground">
						<DownloadIcon class="size-4" />
					</span>
					<div class="min-w-0">
						<div class="flex flex-wrap items-center gap-2">
							<h2 class="text-[15px] font-semibold tracking-tight">
								{t('operations.system.core_update')}
							</h2>
							{@render pill(info.state.channel === 'alpha' ? 'info' : 'neutral', channelLabel)}
							{@render pill(updateStatus.tone, updateStatus.label)}
						</div>
						<p class="mt-1 text-xs text-muted-foreground">
							{t('operations.system.update_description', {
								channel: channelLabel,
								image: info.image_repo,
							})}
						</p>
					</div>
				</div>
				<Button
					variant="outline"
					size="sm"
					class="w-full sm:w-auto sm:shrink-0"
					onclick={check}
					disabled={checking || info.update_interval_secs === 0}>
					<RefreshCwIcon class={checking ? 'size-4 motion-safe:animate-spin' : 'size-4'} />
					{checking ? t('operations.system.checking') : t('operations.system.check_now')}
				</Button>
			</header>

			<div class="flex flex-col gap-4 p-4">
				<dl class="grid grid-cols-2 gap-2.5 lg:grid-cols-4">
					{@render fact(t('operations.system.check_interval'), checkIntervalLabel)}
					<div class="lp-inset flex min-w-0 flex-col gap-0.5 rounded-lg bg-muted/60 px-3 py-2.5">
						<dt class="font-mono text-[11.5px] text-muted-foreground">
							{t('operations.system.last_check')}
						</dt>
						<dd class="flex items-center gap-1.5 font-mono text-[13px] font-medium">
							<ClockIcon class="size-3 shrink-0 text-muted-foreground" />
							{lastCheckLabel}
						</dd>
					</div>
					{@render fact(t('operations.system.current_version'), runningVersion || '—')}
					{@render fact(t('operations.system.latest_version'), info.state.latest_version || '—')}
				</dl>

				<div class="grid gap-2.5 text-sm sm:grid-cols-3">
					{#each serviceStatuses as service (service.name)}
						<div class="min-w-0 rounded-xl border bg-card px-3 py-2.5">
							<div class="flex items-center gap-2">
								<Tooltip.Provider>
									<Tooltip.Root>
										<Tooltip.Trigger aria-label={`${service.name}: ${service.description}`}>
											<span
												class="block size-2 rounded-full {toneDot(
													service.available ? 'ok' : 'danger',
												)} {service.available ? 'motion-safe:animate-pulse' : ''}"></span>
										</Tooltip.Trigger>
										<Tooltip.Content>{service.description}</Tooltip.Content>
									</Tooltip.Root>
								</Tooltip.Provider>
								<p class="truncate text-[13px] font-medium">{service.name}</p>
							</div>
							<p class="mt-1 font-mono text-xs break-all text-muted-foreground">
								{service.currentTag || '—'} → {service.targetTag || '—'}
							</p>
							<p class="mt-0.5 flex items-center gap-1 font-mono text-xs text-muted-foreground">
								<ClockIcon class="size-3 shrink-0" aria-hidden="true" />
								<span class="sr-only">{t('operations.debug.uptime')}:</span>
								{uptimeLabel(service.uptimeSecs)}
							</p>
						</div>
					{/each}
				</div>

				{#if info.state.core_services.caddy.systemd_managed}
					<p class="rounded-xl border border-dashed p-3 text-sm text-muted-foreground">
						{t('operations.system.caddy_systemd')}
					</p>
				{/if}

				{#if !info.state.core_services.supported}
					<p class="rounded-xl border border-dashed p-3 text-sm text-muted-foreground">
						{t('operations.system.core_services_unsupported')}
					</p>
				{/if}

				{#if info.state.last_check_note}
					<p class="lp-inset rounded-xl bg-muted/60 p-3 font-mono text-xs break-words">
						{info.state.last_check_note}
					</p>
				{/if}

				{#if updateReady}
					<div
						class="flex flex-col gap-3 rounded-xl border border-destructive/30 bg-destructive/5 p-4 sm:flex-row">
						<AlertCircleIcon class="mt-0.5 size-5 shrink-0 text-destructive" />
						<div class="min-w-0 flex-1 text-sm">
							<p class="font-medium text-destructive">
								{t('operations.system.version_available', { version: info.state.latest_version })}
							</p>
							<p class="mt-1 text-muted-foreground">
								{t('operations.system.install_description', {
									channel: channelLabel,
									image: `${info.image_repo}:${info.state.latest_version}`,
								})}
							</p>
							<Button
								variant="destructive"
								class="mt-3 w-full sm:w-auto"
								onclick={() => void updateState.startUpdate()}
								disabled={updateState.applying}>
								<RefreshCwIcon
									class={updateState.applying ? 'size-4 motion-safe:animate-spin' : 'size-4'} />
								{updateState.applying
									? t('operations.system.installing')
									: t('operations.system.install_update', { channel: channelLabel })}
							</Button>
						</div>
					</div>
				{:else if info.state.last_check_at && !info.state.last_check_succeeded}
					<div
						class="flex flex-col gap-3 rounded-xl border border-destructive/30 bg-destructive/5 p-4 sm:flex-row">
						<AlertCircleIcon class="mt-0.5 size-5 shrink-0 text-destructive" />
						<div class="min-w-0 text-sm">
							<p class="font-medium text-destructive">
								{t('operations.system.latest_check_failed')}
							</p>
							<p class="mt-1 text-muted-foreground">
								{t('operations.system.latest_check_failed_description', {
									channel: channelLabel,
								})}
							</p>
						</div>
					</div>
				{:else if info.state.last_check_succeeded}
					<div
						class="flex flex-col gap-3 rounded-xl border border-success/30 bg-success/5 p-4 sm:flex-row">
						<CheckCircle2Icon class="mt-0.5 size-5 shrink-0 text-success" />
						<div class="min-w-0 text-sm">
							<p class="font-medium">
								{t('operations.system.latest_published', { channel: channelLabel })}
							</p>
							<p class="mt-1 text-muted-foreground">
								{t('operations.system.latest_published_description', { channel: channelLabel })}
							</p>
						</div>
					</div>
				{/if}
			</div>
		</section>
	{:else if loadError}
		<div class="lp-raised rounded-2xl border border-destructive/30 bg-card p-4 text-sm sm:p-5">
			<div class="flex items-center gap-2 font-medium text-destructive">
				<span class="size-2 rounded-full {toneDot('danger')}" aria-hidden="true"></span>
				{t('operations.system.api_unavailable')}
			</div>
			<p class="mt-2 text-muted-foreground">{loadError}</p>
			<Button class="mt-4 w-full sm:w-auto" onclick={load}>{t('operations.system.retry')}</Button>
		</div>
	{/if}
</div>
