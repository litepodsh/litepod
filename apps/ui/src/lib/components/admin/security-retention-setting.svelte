<script lang="ts">
	import { onMount } from 'svelte';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import {
		getSecurityEventRetention,
		updateSecurityEventRetention,
		type SecurityEventRetention,
	} from '#lib/clients/admin-security-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let current = $state<SecurityEventRetention | null>(null);
	let days = $state(90);
	let saving = $state(false);
	let invalid = $state(false);

	onMount(async () => {
		try {
			current = await getSecurityEventRetention();
			days = current.retention_days;
		} catch (error) {
			toast.error(error instanceof Error ? error.message : t('operations.security.retention_failed'));
		}
	});

	async function save() {
		if (!current) return;
		invalid = !Number.isInteger(days) || days < current.default_days || days > current.max_days;
		if (invalid || days === current.retention_days) return;
		saving = true;
		try {
			current = await updateSecurityEventRetention(days);
			days = current.retention_days;
			toast.success(t('operations.security.retention_updated'));
		} catch (error) {
			days = current.retention_days;
			toast.error(error instanceof Error ? error.message : t('operations.security.retention_failed'));
		} finally {
			saving = false;
		}
	}
</script>

<div class="flex flex-col gap-3 rounded-2xl border bg-card p-4 sm:flex-row sm:items-center sm:justify-between">
	<div class="min-w-0">
		<Label for="security-retention-days">{t('operations.security.retention')}</Label>
		<p class="mt-1 text-sm text-muted-foreground">
			{t('operations.security.retention_description', {
				default: current?.default_days ?? 90,
				max: current?.max_days ?? 365,
			})}
		</p>
		{#if invalid}
			<p role="alert" class="mt-1 text-xs text-destructive">
				{t('operations.security.retention_invalid', {
					default: current?.default_days ?? 90,
					max: current?.max_days ?? 365,
				})}
			</p>
		{/if}
	</div>
	<div class="flex items-center gap-2">
		<Input
			id="security-retention-days"
			type="number"
			min={current?.default_days ?? 90}
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
