<script lang="ts">
	import { onMount } from 'svelte';
	import { Temporal } from '@js-temporal/polyfill';
	import * as Popover from '#lib/components/ui/popover/index.js';
	import OsIcon from '#lib/components/os-icon.svelte';
	import MonitorIcon from '@lucide/svelte/icons/monitor';
	import { getRuntimeStatus, type RuntimeStatus } from '#lib/clients/runtime-client.js';
	import { OS_LABELS } from '#lib/utils/os-label.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let runtime = $state<RuntimeStatus | null>(null);
	let sampledAt = $state<Temporal.Instant | null>(null);
	let now = $state(Temporal.Now.instant());

	let offsetSeconds = $derived(runtime?.utc_offset_seconds ?? 0);
	// The server clock is sampled once and advanced with the local one, so the
	// pill ticks without polling the API.
	let serverNow = $derived(
		runtime && sampledAt
			? Temporal.Instant.from(runtime.server_time).add(now.since(sampledAt))
			: null,
	);
	let uptime = $derived(
		runtime && serverNow ? serverNow.since(Temporal.Instant.from(runtime.started_at)) : null,
	);

	onMount(() => {
		getRuntimeStatus()
			.then((status) => {
				runtime = status;
				sampledAt = now = Temporal.Now.instant();
			})
			.catch(() => {
				// The header stays usable when the status endpoint is unavailable.
			});
		const clock = window.setInterval(() => (now = Temporal.Now.instant()), 1_000);
		return () => window.clearInterval(clock);
	});

	function describeHost(status: RuntimeStatus): string {
		const distribution = status.host_distribution
			? ` (${OS_LABELS[status.host_distribution] ?? status.host_distribution})`
			: '';
		const server = t('common.page_header.host_server', {
			os: OS_LABELS[status.host_os] ?? status.host_os,
			distribution,
			arch: status.host_arch,
		});
		if (!status.podman_host_os) return server;
		return `${server} · ${t('common.page_header.host_podman', {
			os: status.podman_host_os,
			arch: status.podman_host_arch ?? '',
		})}`.trimEnd();
	}

	function zoneOffset(seconds: number) {
		const sign = seconds >= 0 ? '+' : '-';
		const abs = Math.abs(seconds);
		return `${sign}${String(Math.floor(abs / 3_600)).padStart(2, '0')}:${String(Math.floor((abs % 3_600) / 60)).padStart(2, '0')}`;
	}

	const clockFormat: Intl.DateTimeFormatOptions = {
		hour: '2-digit',
		minute: '2-digit',
		second: '2-digit',
	};

	function formatUptime(duration: Temporal.Duration | null) {
		const minutes = Math.max(0, Math.floor(duration?.total({ unit: 'minutes' }) ?? 0));
		const days = Math.floor(minutes / 1_440);
		const hours = Math.floor((minutes % 1_440) / 60);
		return [days ? `${days}d` : null, hours ? `${hours}h` : null, `${minutes % 60}m`]
			.filter(Boolean)
			.join(' ');
	}
</script>

{#if runtime}
	{@const local = now.toZonedDateTimeISO(Temporal.Now.timeZoneId())}
	<Popover.Root>
		<Popover.Trigger
			class="flex h-8 items-center gap-2 rounded-lg border border-foreground/8 bg-background/60 ps-2 pe-2.5 text-xs text-muted-foreground tabular-nums transition-colors outline-none hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring max-lg:hidden">
			<span class="relative grid size-4 place-items-center">
				<OsIcon os={runtime.host_os} distribution={runtime.host_distribution} class="size-4" />
				<span
					class="absolute -end-0.5 -bottom-0.5 size-1.5 rounded-full bg-emerald-500 ring-2 ring-card"
				></span>
			</span>
			<span>{t('common.page_header.up', { duration: formatUptime(uptime) })}</span>
		</Popover.Trigger>
		<Popover.Content align="end" sideOffset={8} class="w-72 rounded-2xl p-0 text-xs">
			<div class="flex items-center gap-3 border-b border-foreground/8 p-3">
				<span class="grid size-9 shrink-0 place-items-center rounded-xl bg-muted">
					<OsIcon os={runtime.host_os} distribution={runtime.host_distribution} class="size-5" />
				</span>
				<div class="min-w-0">
					<p class="text-sm font-medium text-foreground">
						{t('common.page_header.up', { duration: formatUptime(uptime) })}
					</p>
					<p class="truncate text-muted-foreground">{describeHost(runtime)}</p>
				</div>
			</div>
			<div class="grid gap-2 p-3 tabular-nums">
				<div class="flex items-center gap-2">
					<OsIcon
						os={runtime.host_os}
						distribution={runtime.host_distribution}
						class="size-3.5 text-muted-foreground" />
					<span>
						{serverNow
							? t('common.page_header.server_time', {
									time: serverNow
										.toZonedDateTimeISO(zoneOffset(offsetSeconds))
										.toPlainTime()
										.toLocaleString(undefined, clockFormat),
									offset: `UTC${zoneOffset(offsetSeconds)}`,
								})
							: ''}
					</span>
				</div>
				<div class="flex items-center gap-2">
					<MonitorIcon class="size-3.5 text-muted-foreground" aria-hidden="true" />
					<span>
						{t('common.page_header.client_time', {
							time: `${local.toLocaleString(undefined, clockFormat)} (UTC${local.offset})`,
						})}
					</span>
				</div>
			</div>
		</Popover.Content>
	</Popover.Root>
{/if}
