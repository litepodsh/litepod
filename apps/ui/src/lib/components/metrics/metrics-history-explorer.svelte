<script lang="ts">
	import CalendarRangeIcon from '@lucide/svelte/icons/calendar-range';
	import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
	import UndoIcon from '@lucide/svelte/icons/undo-2';
	import RotateCcwIcon from '@lucide/svelte/icons/rotate-ccw';
	import { getLocalTimeZone, today } from '@internationalized/date';
	import type { DateRange } from 'bits-ui';
	import CellChart from '#lib/components/custom/cell-chart.svelte';
	import CapsuleTabs from '#lib/components/custom/capsule-tabs.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Popover from '#lib/components/ui/popover/index.js';
	import { RangeCalendar } from '#lib/components/ui/range-calendar/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import {
		HISTORY_RANGES,
		formatBytes,
		historyGrid,
		type MetricsHistory,
		type MetricsRange,
		type MetricsWindow,
	} from '#lib/clients/container-metrics-client.js';
	import { i18n, t } from '#lib/i18n/index.svelte.js';

	// Stored history of a server (CPU, RAM, disk) or an application (CPU, memory).
	// Pick a preset or a date range, then drag across any chart to zoom into it.
	let {
		load,
		host = false,
		memoryFallback = null,
		initialRange = '24h',
	}: {
		load: (window: MetricsWindow) => Promise<MetricsHistory>;
		/** A server: shows disk and reads RAM against the host's capacity. */
		host?: boolean;
		/** RAM capacity to scale against when no host sample says otherwise. */
		memoryFallback?: number | null;
		initialRange?: MetricsRange;
	} = $props();

	const DAY = 24 * 3600_000;

	// svelte-ignore state_referenced_locally
	let range = $state<MetricsRange>(initialRange);
	// Each zoom pushes a window; back pops one, a preset clears them all.
	let zooms = $state<{ from: number; to: number }[]>([]);
	const active = $derived<MetricsWindow>(zooms.at(-1) ?? range);

	let history = $state<MetricsHistory | null>(null);
	let error = $state(false);
	$effect(() => {
		const selected = active;
		let cancelled = false;
		error = false;
		load(selected).then(
			(result) => {
				if (!cancelled) history = result;
			},
			() => {
				if (!cancelled) error = true;
			},
		);
		return () => (cancelled = true);
	});

	const grid = $derived(history ? historyGrid(history) : null);
	const cols = $derived(grid?.starts.length ?? 0);

	let hovered = $state<number | null>(null);
	let dragFrom = $state<number | null>(null);
	const selection = $derived<[number, number] | null>(
		dragFrom !== null && hovered !== null
			? [Math.min(dragFrom, hovered), Math.max(dragFrom, hovered)]
			: null,
	);

	function startDrag(event: PointerEvent) {
		if (event.button !== 0 || hovered === null) return;
		dragFrom = hovered;
		globalThis.addEventListener('pointerup', endDrag, { once: true });
	}

	function endDrag() {
		const picked = selection;
		dragFrom = null;
		if (!picked || !grid || picked[0] === picked[1]) return;
		const from = grid.starts[picked[0]];
		const to = grid.starts[picked[1]] + grid.bucketMs;
		// Below a few minutes a bucket is a single sample; nothing more to see.
		if (to - from < 5 * 60_000) return;
		zooms = [...zooms, { from, to }];
	}

	function pickRange() {
		zooms = [];
		calendar = undefined;
	}

	// Calendar days are local; the window runs from the first day's midnight to the
	// midnight after the last.
	let calendar = $state<DateRange | undefined>();
	let calendarOpen = $state(false);
	const zone = getLocalTimeZone();
	const maxDay = today(zone);
	const minDay = maxDay.subtract({ days: 365 });
	function pickDays(value: DateRange | undefined) {
		if (!value?.start || !value.end) return;
		const from = value.start.toDate(zone).getTime();
		const to = Math.min(Date.now(), value.end.add({ days: 1 }).toDate(zone).getTime());
		zooms = [...zooms, { from, to }];
		calendarOpen = false;
	}

	function stamp(ms: number, withTime: boolean, withDate: boolean): string {
		return new Date(ms).toLocaleString(i18n.locale, {
			...(withDate ? { day: 'numeric', month: 'short' } : {}),
			...(withTime ? { hour: '2-digit', minute: '2-digit' } : {}),
		});
	}
	const span = $derived(history ? history.to - history.from : 0);
	function columnLabel(i: number): string {
		if (!grid) return '';
		const from = grid.starts[i];
		const to = from + grid.bucketMs;
		if (grid.bucketMs >= DAY) return `${stamp(from, false, true)} – ${stamp(to, false, true)}`;
		const withDate = span > DAY;
		return `${stamp(from, true, withDate)} – ${stamp(to, true, false)}`;
	}
	const windowLabel = $derived(
		history
			? `${stamp(history.from, span <= 7 * DAY, true)} – ${stamp(history.to, span <= 7 * DAY, true)}`
			: '',
	);

	const known = (values: (number | null)[]) => values.filter((v): v is number => v !== null);
	const peak = (values: (number | null)[]) => {
		const list = known(values);
		return list.length ? Math.max(...list) : null;
	};
	const latest = (values: (number | null)[]) => known(values).at(-1) ?? null;
	const filled = (values: (number | null)[]) => values.map((v) => v ?? 0);

	const memCaps = $derived(grid && host && grid.hostMemory ? grid.memCap : null);
	const memMax = $derived.by(() => {
		if (!grid) return 1;
		const cap = memCaps ? peak(memCaps) : host ? memoryFallback : null;
		return Math.max(cap ?? 0, peak(grid.mem) ?? 0, 1) * (cap ? 1 : 1.08);
	});
	const diskMax = $derived(grid ? Math.max(peak(grid.diskCap) ?? 0, peak(grid.disk) ?? 0, 1) : 1);
	const hasDisk = $derived(host && !!grid && grid.disk.some((v) => v !== null));

	function bytesOf(value: number | null, cap: number | null): string {
		if (value === null) return t('servers.card.no_data');
		if (!cap) return formatBytes(value);
		return `${formatBytes(value)} / ${formatBytes(cap)} · ${Math.round((value / cap) * 100)}%`;
	}
	const cpuText = (v: number | null) =>
		v === null ? t('servers.card.no_data') : `${v.toFixed(1)}%`;

	const rangeItems = $derived(
		HISTORY_RANGES.map((value) => ({ value, label: t(`common.metrics_history.range_${value}`) })),
	);
</script>

{#snippet chart(
	title: string,
	values: (number | null)[],
	summary: string,
	at: (i: number) => string,
	max: number | undefined,
	caps: (number | null)[] | null,
	tone: 'primary' | 'success' | 'warning',
)}
	<div class="flex min-w-0 flex-col gap-1.5">
		<div class="flex items-baseline justify-between gap-3 text-xs text-muted-foreground">
			<span class="font-medium text-foreground">{title}</span>
			<b class="truncate font-mono text-[12.5px] font-medium text-foreground tabular-nums">
				{hovered !== null ? at(hovered) : summary}
			</b>
		</div>
		<!-- svelte-ignore a11y_no_static_element_interactions (drag-to-zoom is a pointer shortcut; presets and the calendar cover keyboards) -->
		<div class="cursor-crosshair touch-none select-none" onpointerdown={startDrag}>
			<CellChart
				values={filled(values)}
				{cols}
				rows={8}
				{max}
				caps={caps ?? undefined}
				{selection}
				{tone}
				onhover={(i) => (hovered = i)}
				ariaLabel={title} />
		</div>
	</div>
{/snippet}

<div class="flex flex-col gap-4" data-testid="metrics-history-explorer">
	<div class="flex flex-wrap items-center gap-2">
		<CapsuleTabs
			items={rangeItems}
			bind:value={range}
			onchange={pickRange}
			label={t('common.metrics_history.range')} />
		<Popover.Root bind:open={calendarOpen}>
			<Popover.Trigger>
				{#snippet child({ props })}
					<Button {...props} variant="outline" size="sm">
						<CalendarRangeIcon class="size-4" />
						{t('common.metrics_history.pick_dates')}
					</Button>
				{/snippet}
			</Popover.Trigger>
			<Popover.Content class="w-auto p-0" align="start">
				<RangeCalendar
					bind:value={calendar}
					onValueChange={pickDays}
					minValue={minDay}
					maxValue={maxDay}
					locale={i18n.locale}
					numberOfMonths={1} />
			</Popover.Content>
		</Popover.Root>
		{#if zooms.length}
			<Button variant="ghost" size="sm" onclick={() => (zooms = zooms.slice(0, -1))}>
				<UndoIcon class="size-4" />
				{t('common.metrics_history.back')}
			</Button>
			<Button variant="ghost" size="sm" onclick={pickRange}>
				<RotateCcwIcon class="size-4" />
				{t('common.metrics_history.reset')}
			</Button>
		{/if}
	</div>

	<div class="flex flex-wrap items-baseline justify-between gap-2 text-xs text-muted-foreground">
		<span class="font-mono tabular-nums">
			{hovered !== null && grid ? columnLabel(hovered) : windowLabel}
		</span>
		<span>{t('common.metrics_history.drag_hint')}</span>
	</div>

	{#if error}
		<p
			role="alert"
			class="flex items-center gap-2 rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive">
			<CircleAlertIcon class="size-3.5 flex-none" />
			{t('common.metrics_history.error')}
		</p>
	{:else if !grid}
		<div class="flex flex-col gap-4">
			<Skeleton class="h-20 rounded-md" />
			<Skeleton class="h-20 rounded-md" />
		</div>
	{:else if !grid.cpu.some((v) => v !== null) && !grid.mem.some((v) => v !== null) && !hasDisk}
		<p class="rounded-xl border border-dashed p-4 text-center text-sm text-muted-foreground">
			{t('common.metrics_history.empty')}
		</p>
	{:else}
		<div class="flex flex-col gap-5">
			{@render chart(
				t('common.metrics_history.cpu'),
				grid.cpu,
				t('common.metrics_history.peak', { value: cpuText(peak(grid.cpu)) }),
				(i) => cpuText(grid.cpu[i]),
				Math.max(100, peak(grid.cpu) ?? 0),
				null,
				'primary',
			)}
			{@render chart(
				t(host && grid.hostMemory ? 'common.metrics_history.ram' : 'common.metrics_history.memory'),
				grid.mem,
				bytesOf(latest(grid.mem), memCaps ? latest(memCaps) : host ? memoryFallback : null),
				(i) => bytesOf(grid.mem[i], memCaps ? memCaps[i] : host ? memoryFallback : null),
				memMax,
				memCaps,
				'success',
			)}
			{#if hasDisk}
				{@render chart(
					t('common.metrics_history.disk'),
					grid.disk,
					bytesOf(latest(grid.disk), latest(grid.diskCap)),
					(i) => bytesOf(grid.disk[i], grid.diskCap[i]),
					diskMax,
					grid.diskCap,
					(latest(grid.disk) ?? 0) / (latest(grid.diskCap) ?? Infinity) >= 0.85
						? 'warning'
						: 'primary',
				)}
			{/if}
		</div>
	{/if}
</div>
