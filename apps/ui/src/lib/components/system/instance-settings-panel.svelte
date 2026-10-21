<script lang="ts">
	import { onMount } from 'svelte';
	import CertificateFields from '#lib/components/custom/certificate-fields.svelte';
	import DeleteConfirmControl from '#lib/components/custom/delete-confirm-control.svelte';
	import SoftSwitch from '#lib/components/custom/soft-switch.svelte';
	import SettingsSection from '#lib/components/projects/settings-section.svelte';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import {
		getServerDomainSettings,
		restartInfrastructure,
		updateServerDomainSettings,
		type CertificateProvider,
		type InfrastructureTarget,
	} from '#lib/clients/server-settings-client.js';
	import {
		getScheduledTaskHistorySettings,
		updateScheduledTaskHistorySettings,
	} from '#lib/clients/scheduled-task-settings-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import RotateCwIcon from '@lucide/svelte/icons/rotate-cw';

	// Instance-wide settings: the console's own domain, the infrastructure it runs
	// on and how long scheduled-task history is kept. Sysadmin only, like the API.
	const infrastructureTargets: {
		key: InfrastructureTarget;
		label: string;
		description: string;
	}[] = [
		{ key: 'caddy', label: 'Caddy', description: t('account.settings.caddy_description') },
		{ key: 'api', label: 'API', description: t('account.settings.api_description') },
	];

	let loading = $state(true);
	let domain = $state('');
	let email = $state('');
	let https = $state(true);
	let certificateProvider = $state<CertificateProvider>('automatic');
	let certificateId = $state('');
	let certificateName = $state('');
	let savingDomain = $state(false);
	let historyRetentionDays = $state(60);
	let savingHistoryRetention = $state(false);
	let restarting = $state<Record<InfrastructureTarget, boolean>>({ caddy: false, api: false });
	let confirmTarget = $state<InfrastructureTarget | null>(null);

	onMount(async () => {
		try {
			const [serverSettings, taskHistorySettings] = await Promise.all([
				getServerDomainSettings(),
				getScheduledTaskHistorySettings(),
			]);
			domain = serverSettings.domain ?? '';
			email = serverSettings.email ?? '';
			https = serverSettings.https;
			certificateProvider = serverSettings.certificate_provider;
			certificateId = serverSettings.certificate_id ?? '';
			certificateName = serverSettings.certificate_name ?? '';
			historyRetentionDays = taskHistorySettings.history_retention_days;
		} catch (err) {
			toast.error(err instanceof Error ? err.message : t('account.settings.errors.save_domain'));
		} finally {
			loading = false;
		}
	});

	async function saveServerDomain() {
		savingDomain = true;
		try {
			const saved = await updateServerDomainSettings({
				domain,
				email,
				https,
				certificate_provider: https ? certificateProvider : 'automatic',
				...(https && certificateProvider === 'custom' ? { certificate_id: certificateId } : {}),
			});
			domain = saved.domain ?? '';
			email = saved.email ?? '';
			https = saved.https;
			certificateProvider = saved.certificate_provider;
			certificateId = saved.certificate_id ?? '';
			certificateName = saved.certificate_name ?? '';
			toast.success(t('account.settings.success.domain_saved'));
		} catch (err) {
			toast.error(err instanceof Error ? err.message : t('account.settings.errors.save_domain'));
		} finally {
			savingDomain = false;
		}
	}

	async function confirmRestart() {
		const target = confirmTarget;
		if (!target) return;
		restarting[target] = true;
		try {
			await restartInfrastructure(target);
			toast.success(t('account.settings.success.restart_triggered', { target }));
		} catch (err) {
			toast.error(
				err instanceof Error ? err.message : t('account.settings.errors.restart', { target }),
			);
		} finally {
			restarting[target] = false;
			confirmTarget = null;
		}
	}

	async function saveHistoryRetention() {
		savingHistoryRetention = true;
		try {
			const saved = await updateScheduledTaskHistorySettings(historyRetentionDays);
			historyRetentionDays = saved.history_retention_days;
			toast.success(t('account.settings.success.task_history_saved'));
		} catch (err) {
			toast.error(
				err instanceof Error ? err.message : t('account.settings.errors.save_task_history'),
			);
		} finally {
			savingHistoryRetention = false;
		}
	}
</script>

{#snippet saveButton(saving: boolean, disabled: boolean)}
	<div class="flex justify-end">
		<Button type="submit" {disabled}
			>{saving ? t('account.common.saving') : t('account.common.save')}</Button>
	</div>
{/snippet}

<div class="flex max-w-[820px] flex-col gap-4">
	<SettingsSection
		title={t('account.settings.server_domain')}
		description={t('account.settings.server_domain_description')}>
		<form
			class="flex flex-col gap-4"
			onsubmit={(event) => {
				event.preventDefault();
				saveServerDomain();
			}}>
			<div class="grid gap-4 sm:grid-cols-2">
				<div class="grid content-start gap-1.5">
					<Label for="server-domain">{t('account.settings.domain')}</Label>
					<Input
						id="server-domain"
						bind:value={domain}
						placeholder="litepod.example.com"
						autocomplete="url"
						class="font-mono"
						disabled={loading || savingDomain}
						required />
					<p class="text-xs text-muted-foreground">{t('account.settings.domain_hint')}</p>
				</div>
				<div class="grid content-start gap-1.5">
					<Label for="certificate-email">{t('account.settings.certificate_email')}</Label>
					<Input
						id="certificate-email"
						type="email"
						bind:value={email}
						placeholder="ops@example.com"
						autocomplete="email"
						disabled={loading || savingDomain}
						required />
					<p class="text-xs text-muted-foreground">
						{t('account.settings.certificate_email_hint')}
					</p>
				</div>
			</div>
			<div class="flex items-center justify-between gap-4">
				<div class="min-w-0">
					<Label for="server-https">HTTPS</Label>
					<p class="mt-1 text-xs text-muted-foreground">{t('account.settings.https_hint')}</p>
				</div>
				<SoftSwitch id="server-https" bind:checked={https} disabled={loading || savingDomain} />
			</div>
			{#if https}
				<div class="grid gap-4">
					<CertificateFields
						bind:provider={certificateProvider}
						bind:certificateId
						{certificateName}
						disabled={loading || savingDomain} />
				</div>
			{/if}
			{@render saveButton(savingDomain, loading || savingDomain)}
		</form>
	</SettingsSection>

	<SettingsSection
		title={t('account.settings.infrastructure')}
		description={t('account.settings.infrastructure_description')}>
		<div class="flex flex-col gap-1.5">
			{#each infrastructureTargets as target (target.key)}
				<div
					class="flex flex-col gap-3 rounded-[10px] border border-foreground/9 px-3 py-2.5 sm:flex-row sm:items-center sm:justify-between">
					<div class="min-w-0">
						<div class="font-medium">{target.label}</div>
						<p class="text-xs text-muted-foreground">{target.description}</p>
					</div>
					<Button
						variant="outline"
						size="sm"
						class="shrink-0 self-start sm:self-auto"
						disabled={restarting[target.key]}
						onclick={() => (confirmTarget = target.key)}>
						<RotateCwIcon
							class={restarting[target.key] ? 'animate-spin motion-reduce:animate-none' : ''} />
						{restarting[target.key]
							? t('account.settings.restarting')
							: t('account.settings.restart')}
					</Button>
				</div>
			{/each}
		</div>
	</SettingsSection>

	<SettingsSection
		title={t('account.settings.task_history')}
		description={t('account.settings.task_history_description')}>
		<form
			class="flex flex-col gap-4"
			onsubmit={(event) => {
				event.preventDefault();
				saveHistoryRetention();
			}}>
			<div class="grid max-w-xs gap-1.5">
				<Label for="task-history-days">{t('account.settings.task_history_retention_days')}</Label>
				<Input
					id="task-history-days"
					type="number"
					min="0"
					max="3650"
					class="font-mono tabular-nums"
					bind:value={historyRetentionDays}
					disabled={loading || savingHistoryRetention} />
				<p class="text-xs text-muted-foreground">
					{t('account.settings.task_history_retention_hint')}
				</p>
			</div>
			{@render saveButton(savingHistoryRetention, loading || savingHistoryRetention)}
		</form>
	</SettingsSection>
</div>

<AlertDialog.Root open={confirmTarget !== null}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title
				>{t('account.settings.restart_confirm_title', { target: confirmTarget })}</AlertDialog.Title>
			<AlertDialog.Description>
				{#if confirmTarget}
					{infrastructureTargets.find((item) => item.key === confirmTarget)?.description}
				{/if}
			</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel
				disabled={confirmTarget !== null && restarting[confirmTarget]}
				onclick={() => (confirmTarget = null)}>{t('account.common.cancel')}</AlertDialog.Cancel>
			<DeleteConfirmControl
				label={t('account.settings.restart')}
				pendingLabel={t('account.settings.restarting')}
				variant="primary"
				disabled={confirmTarget === null || restarting[confirmTarget]}
				onconfirm={confirmRestart} />
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
