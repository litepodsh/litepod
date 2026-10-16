<script lang="ts">
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import {
		getMetricsRetention,
		updateMetricsRetention,
		type MetricsRetention,
	} from '#lib/clients/container-metrics-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let { organizationId }: { organizationId: string } = $props();

	let current = $state<MetricsRetention | null>(null);
	let days = $state<number>(90);
	let saving = $state(false);
	let loadError = $state(false);
	let invalid = $state(false);

	$effect(() => {
		const id = organizationId;
		let cancelled = false;
		current = null;
		loadError = false;
		getMetricsRetention(id).then(
			(result) => {
				if (cancelled) return;
				current = result;
				days = result.retention_days;
			},
			() => {
				if (!cancelled) loadError = true;
			},
		);
		return () => (cancelled = true);
	});

	async function save() {
		if (!current) return;
		const max = current.max_days;
		invalid = !Number.isInteger(days) || days < 1 || days > max;
		if (invalid || days === current.retention_days) return;
		saving = true;
		try {
			current = await updateMetricsRetention(organizationId, days);
			days = current.retention_days;
			toast.success(t('organizations.metrics_retention.updated'));
		} catch (err) {
			days = current.retention_days;
			toast.error(err instanceof Error ? err.message : t('organizations.metrics_retention.failed'));
		} finally {
			saving = false;
		}
	}
</script>

<div
	class="flex flex-col gap-3 lp-raised rounded-2xl border bg-card p-4 sm:flex-row sm:items-center sm:justify-between sm:gap-4"
	data-testid="metrics-retention-setting">
	<div class="min-w-0">
		<Label for="metrics-retention-days">{t('organizations.metrics_retention.title')}</Label>
		<p class="mt-1 text-sm text-muted-foreground">
			{#if loadError}
				{t('organizations.metrics_retention.load_failed')}
			{:else}
				{t('organizations.metrics_retention.description', {
					max: current?.max_days ?? 365,
					default: current?.default_days ?? 90,
				})}
			{/if}
		</p>
		{#if invalid}
			<p role="alert" class="mt-1 text-xs text-destructive">
				{t('organizations.metrics_retention.invalid', { max: current?.max_days ?? 365 })}
			</p>
		{/if}
	</div>
	<div class="flex items-center gap-2">
		<Input
			id="metrics-retention-days"
			type="number"
			min="1"
			max={current?.max_days ?? 365}
			step="1"
			class="w-20"
			aria-invalid={invalid}
			disabled={saving || !current}
			bind:value={days}
			onchange={save} />
		<span class="text-sm text-muted-foreground">{t('organizations.audit_logs.days')}</span>
	</div>
</div>
