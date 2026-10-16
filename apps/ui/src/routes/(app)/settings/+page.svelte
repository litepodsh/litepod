<script lang="ts">
	import CertificatesSettings from '#lib/components/custom/certificates-settings.svelte';
	import CertificateFields from '#lib/components/custom/certificate-fields.svelte';
	import type { CertificateProvider } from '#lib/clients/server-settings-client.js';
	import SkeletonRows from '#lib/components/custom/skeleton-rows.svelte';
	import SettingsSection from '#lib/components/projects/settings-section.svelte';
	import { onDestroy, onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import DeleteConfirmControl from '#lib/components/custom/delete-confirm-control.svelte';
	import { tabIndexFromKeydown } from '#lib/utils/tab-shortcut.js';
	import {
		clearScopeHero,
		clearShellTabs,
		setScopeHero,
		setShellTabs,
	} from '#lib/state/shell.svelte.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import SoftSwitch from '#lib/components/custom/soft-switch.svelte';
	import { listOrganizations, type OrganizationDto } from '#lib/clients/organization-client.js';
	import { getMe } from '#lib/clients/auth-client.js';
	import { getSystemVersion } from '#lib/clients/system-client.js';
	import {
		getServerDomainSettings,
		updateServerDomainSettings,
		restartInfrastructure,
		type InfrastructureTarget,
	} from '#lib/clients/server-settings-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import OrganizationIcon from '#lib/components/organization-icon.svelte';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import CpuIcon from '@lucide/svelte/icons/cpu';
	import RotateCwIcon from '@lucide/svelte/icons/rotate-cw';
	import ArrowRightIcon from '@lucide/svelte/icons/arrow-right';
	import ChevronRightIcon from '@lucide/svelte/icons/chevron-right';
	import GaugeIcon from '@lucide/svelte/icons/gauge';
	import ShieldCheckIcon from '@lucide/svelte/icons/shield-check';
	import Building2Icon from '@lucide/svelte/icons/building-2';
	import SlidersHorizontalIcon from '@lucide/svelte/icons/sliders-horizontal';
	import SettingsIcon from '@lucide/svelte/icons/settings';
	import {
		getBuildConfiguration,
		updateBuildConfiguration,
	} from '#lib/clients/build-configuration-client.js';
	import {
		getScheduledTaskHistorySettings,
		updateScheduledTaskHistorySettings,
	} from '#lib/clients/scheduled-task-settings-client.js';

	import {
		getRateLimitSettings,
		updateRateLimitSettings,
		type RateLimitSettings,
	} from '#lib/clients/rate-limit-client.js';

	const infrastructureTargets: {
		key: InfrastructureTarget;
		label: string;
		description: string;
	}[] = [
		{
			key: 'caddy',
			label: 'Caddy',
			description: t('account.settings.caddy_description'),
		},

		{
			key: 'api',
			label: 'API',
			description: t('account.settings.api_description'),
		},
	];

	let organizations = $state<OrganizationDto[]>([]);
	let loading = $state(true);
	let activeTab = $state('general');
	let domain = $state('');
	let email = $state('');
	let https = $state(true);
	let certificateProvider = $state<CertificateProvider>('automatic');
	let certificateId = $state('');
	let certificateName = $state('');
	let savingDomain = $state(false);
	let maxConcurrentBuilds = $state(3);
	let activeBuilds = $state(0);
	let savingBuildConfiguration = $state(false);
	let historyRetentionDays = $state(60);
	let savingHistoryRetention = $state(false);
	let rateLimit = $state<RateLimitSettings>({
		enabled: true,
		global_per_second: 20,
		global_burst: 40,
		auth_per_second: 5,
		auth_burst: 10,
		auth_max_attempts: 5,
		auth_ban_minutes: 15,
	});
	let savingRateLimit = $state(false);
	let restarting = $state<Record<InfrastructureTarget, boolean>>({
		caddy: false,
		api: false,
	});
	let confirmTarget = $state<InfrastructureTarget | null>(null);

	// Nothing renders until the guard has resolved. Painting the admin shell first
	// and redirecting afterwards shows the page to whoever asked for it, however
	// briefly — the same defect the report raised against /system and /maintenance.
	let guardResolved = $state(false);
	// Every settings endpoint except `listOrganizations` refuses a non-sysadmin
	// (`api/src/handler/settings.rs`), regardless of edition. Hiding those tabs
	// matches the API instead of rendering three panels that can only 403.
	let isSysadmin = $state(false);

	const organizationsTab = {
		value: 'organizations',
		label: t('account.settings.tabs.organizations'),
		icon: Building2Icon,
	};
	const visibleTabs = $derived(
		isSysadmin
			? [
					{
						value: 'general',
						label: t('account.settings.tabs.general'),
						icon: SlidersHorizontalIcon,
					},
					{
						value: 'certificates',
						label: t('account.certificates.title'),
						icon: ShieldCheckIcon,
					},
					{ value: 'builds', label: t('account.settings.tabs.builds'), icon: CpuIcon },
					{ value: 'rate-limit', label: t('account.settings.tabs.rate_limit'), icon: GaugeIcon },
					organizationsTab,
				]
			: [organizationsTab],
	);
	$effect(() => {
		if (!guardResolved) return;
		const tab = page.url.searchParams.get('tab');
		if (tab && visibleTabs.some((item) => item.value === tab)) activeTab = tab;
	});

	// The shell draws the title card and the tab capsule; only once the guard has passed.
	$effect(() => {
		if (!guardResolved) return;
		setScopeHero({
			title: t('account.settings.title'),
			avatar: heroAvatar,
			meta: heroMeta,
			stats: [
				{
					label: t('account.settings.tabs.organizations'),
					value: loading ? '…' : String(organizations.length),
				},
			],
		});
		setShellTabs(visibleTabs, activeTab, updateTab);
	});
	onDestroy(() => {
		clearScopeHero();
		clearShellTabs();
	});

	onMount(async () => {
		try {
			const [me, systemVersion] = await Promise.all([getMe(), getSystemVersion()]);
			isSysadmin = me.is_sysadmin;
			if (systemVersion.is_cloud && !isSysadmin) {
				toast.error(t('account.settings.errors.admin_only'));
				goto('/home');
				return;
			}
			guardResolved = true;
			if (!isSysadmin) {
				// Only the organizations tab is reachable for this user.
				activeTab = 'organizations';
				organizations = await listOrganizations();
				return;
			}
			const [
				organizationList,
				serverSettings,
				buildConfiguration,
				rateLimitSettings,
				taskHistorySettings,
			] = await Promise.all([
				listOrganizations(),
				getServerDomainSettings(),
				getBuildConfiguration(),
				getRateLimitSettings(),
				getScheduledTaskHistorySettings(),
			]);
			organizations = organizationList;
			domain = serverSettings.domain ?? '';
			email = serverSettings.email ?? '';
			https = serverSettings.https;
			certificateProvider = serverSettings.certificate_provider;
			certificateId = serverSettings.certificate_id ?? '';
			certificateName = serverSettings.certificate_name ?? '';
			maxConcurrentBuilds = buildConfiguration.max_concurrent_builds;
			activeBuilds = buildConfiguration.active_builds;
			rateLimit = rateLimitSettings;
			historyRetentionDays = taskHistorySettings.history_retention_days;
		} catch (err) {
			toast.error(
				err instanceof Error ? err.message : t('account.settings.errors.load_organizations'),
			);
		} finally {
			loading = false;
		}
	});

	function updateTab(tab: string) {
		activeTab = tab;
		goto(`?tab=${tab}`, { replace: true, reset: false });
	}

	function handleTabShortcut(event: KeyboardEvent) {
		const index = tabIndexFromKeydown(event, visibleTabs.length);
		if (index === null) return;
		event.preventDefault();
		updateTab(visibleTabs[index].value);
	}

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

	async function saveBuildConfiguration() {
		savingBuildConfiguration = true;
		try {
			const saved = await updateBuildConfiguration(maxConcurrentBuilds);
			maxConcurrentBuilds = saved.max_concurrent_builds;
			activeBuilds = saved.active_builds;
			toast.success(t('account.settings.success.builds_saved'));
		} catch (err) {
			toast.error(err instanceof Error ? err.message : t('account.settings.errors.save_builds'));
		} finally {
			savingBuildConfiguration = false;
		}
	}

	async function saveRateLimit() {
		savingRateLimit = true;
		try {
			rateLimit = await updateRateLimitSettings(rateLimit);
			toast.success(t('account.settings.success.rate_limit_saved'));
		} catch (err) {
			toast.error(
				err instanceof Error ? err.message : t('account.settings.errors.save_rate_limit'),
			);
		} finally {
			savingRateLimit = false;
		}
	}
</script>

<svelte:window onkeydown={handleTabShortcut} />

{#snippet heroAvatar()}
	<span class="grid size-full place-items-center bg-muted"><SettingsIcon class="size-5" /></span>
{/snippet}

{#snippet heroMeta()}
	<span class="max-w-2xl">{t('account.settings.description')}</span>
{/snippet}

{#snippet saveButton(saving: boolean, disabled: boolean)}
	<div class="flex justify-end">
		<Button type="submit" {disabled}
			>{saving ? t('account.common.saving') : t('account.common.save')}</Button>
	</div>
{/snippet}

{#if guardResolved}
	<div class="flex w-full min-w-0 flex-col gap-6 px-4 py-6 sm:px-6">
		<div class="flex max-w-[820px] flex-col gap-4">
			{#if isSysadmin && activeTab === 'general'}
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
							<SoftSwitch
								id="server-https"
								bind:checked={https}
								disabled={loading || savingDomain} />
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
										class={restarting[target.key]
											? 'animate-spin motion-reduce:animate-none'
											: ''} />
									{restarting[target.key]
										? t('account.settings.restarting')
										: t('account.settings.restart')}
								</Button>
							</div>
						{/each}
					</div>
				</SettingsSection>

				<SettingsSection
					title={t('account.backups.title')}
					description={t('account.settings.backups_description')}>
					<div>
						<Button href="/system?tab=backups" variant="outline" size="sm">
							{t('account.settings.manage')}
							<ArrowRightIcon />
						</Button>
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
							<Label for="task-history-days"
								>{t('account.settings.task_history_retention_days')}</Label>
							<Input
								id="task-history-days"
								type="number"
								min="0"
								max="3650"
								class="font-mono tabular-nums"
								bind:value={historyRetentionDays}
								disabled={savingHistoryRetention} />
							<p class="text-xs text-muted-foreground">
								{t('account.settings.task_history_retention_hint')}
							</p>
						</div>
						{@render saveButton(savingHistoryRetention, savingHistoryRetention)}
					</form>
				</SettingsSection>
			{:else if isSysadmin && activeTab === 'certificates'}
				<CertificatesSettings />
			{:else if isSysadmin && activeTab === 'builds'}
				<SettingsSection
					title={t('account.settings.tabs.builds')}
					description={t('account.settings.builds_description')}>
					<form
						class="flex flex-col gap-4"
						onsubmit={(event) => {
							event.preventDefault();
							saveBuildConfiguration();
						}}>
						<div class="grid max-w-xs gap-1.5">
							<Label for="max-builds">{t('account.settings.max_concurrent_builds')}</Label>
							<Input
								id="max-builds"
								type="number"
								min="1"
								max="64"
								class="font-mono tabular-nums"
								bind:value={maxConcurrentBuilds}
								disabled={savingBuildConfiguration} />
							<p class="text-xs text-muted-foreground">
								{t('account.settings.builds_active_hint', { active: activeBuilds })}
							</p>
						</div>
						{@render saveButton(savingBuildConfiguration, savingBuildConfiguration)}
					</form>
				</SettingsSection>
			{:else if isSysadmin && activeTab === 'rate-limit'}
				<form
					class="flex flex-col gap-4"
					onsubmit={(event) => {
						event.preventDefault();
						saveRateLimit();
					}}>
					<SettingsSection
						title={t('account.settings.tabs.rate_limit')}
						description={t('account.settings.rate_limit_description')}>
						<div class="flex items-center justify-between gap-4">
							<Label for="rate-limit-enabled"
								>{rateLimit.enabled
									? t('account.common.enabled')
									: t('account.common.disabled')}</Label>
							<SoftSwitch
								id="rate-limit-enabled"
								bind:checked={rateLimit.enabled}
								disabled={savingRateLimit} />
						</div>
					</SettingsSection>
					<SettingsSection
						title={t('account.settings.global_requests')}
						description={t('account.settings.global_requests_hint')}>
						<div class="grid gap-4 sm:grid-cols-2">
							<div class="grid content-start gap-1.5">
								<Label for="global-per-second">{t('account.settings.requests_per_second')}</Label>
								<Input
									id="global-per-second"
									type="number"
									min="1"
									class="font-mono tabular-nums"
									bind:value={rateLimit.global_per_second}
									disabled={savingRateLimit} />
							</div>
							<div class="grid content-start gap-1.5">
								<Label for="global-burst">{t('account.settings.burst')}</Label>
								<Input
									id="global-burst"
									type="number"
									min="1"
									class="font-mono tabular-nums"
									bind:value={rateLimit.global_burst}
									disabled={savingRateLimit} />
							</div>
						</div>
					</SettingsSection>
					<SettingsSection
						title={t('account.settings.auth_requests')}
						description={t('account.settings.auth_requests_hint')}>
						<div class="grid gap-4 sm:grid-cols-2">
							<div class="grid content-start gap-1.5">
								<Label for="auth-per-second">{t('account.settings.requests_per_second')}</Label>
								<Input
									id="auth-per-second"
									type="number"
									min="1"
									class="font-mono tabular-nums"
									bind:value={rateLimit.auth_per_second}
									disabled={savingRateLimit} />
							</div>
							<div class="grid content-start gap-1.5">
								<Label for="auth-burst">{t('account.settings.burst')}</Label>
								<Input
									id="auth-burst"
									type="number"
									min="1"
									class="font-mono tabular-nums"
									bind:value={rateLimit.auth_burst}
									disabled={savingRateLimit} />
							</div>
						</div>
					</SettingsSection>
					<SettingsSection
						title={t('account.settings.login_ban')}
						description={t('account.settings.login_ban_hint')}>
						<div class="grid gap-4 sm:grid-cols-2">
							<div class="grid content-start gap-1.5">
								<Label for="auth-max-attempts"
									>{t('account.settings.failed_attempts_before_ban')}</Label>
								<Input
									id="auth-max-attempts"
									type="number"
									min="1"
									class="font-mono tabular-nums"
									bind:value={rateLimit.auth_max_attempts}
									disabled={savingRateLimit} />
							</div>
							<div class="grid content-start gap-1.5">
								<Label for="auth-ban-minutes">{t('account.settings.ban_duration_minutes')}</Label>
								<Input
									id="auth-ban-minutes"
									type="number"
									min="1"
									class="font-mono tabular-nums"
									bind:value={rateLimit.auth_ban_minutes}
									disabled={savingRateLimit} />
							</div>
						</div>
					</SettingsSection>
					{@render saveButton(savingRateLimit, savingRateLimit)}
				</form>
			{:else if activeTab === 'organizations'}
				<SettingsSection title={t('account.settings.tabs.organizations')}>
					{#if loading}
						<SkeletonRows
							rows={3}
							height="h-12"
							label={t('account.settings.loading_organizations')} />
					{:else if organizations.length === 0}
						<p class="rounded-[10px] bg-muted/40 px-3 py-4 text-sm text-muted-foreground">
							{t('account.settings.organizations_empty')}
						</p>
					{:else}
						<div class="flex flex-col gap-1.5">
							{#each organizations as organization (organization.id)}
								<a
									href="/organizations/{organization.id}"
									class="group flex items-center justify-between gap-3 rounded-[10px] border border-foreground/9 px-3 py-2.5 transition-colors hover:bg-muted/50 motion-reduce:transition-none">
									<span class="flex min-w-0 items-center gap-3">
										<span
											class="{organization.icon_bg
												? ''
												: 'bg-muted'} lp-raised flex size-8 shrink-0 items-center justify-center overflow-hidden rounded-lg border">
											<OrganizationIcon
												icon={organization.icon}
												bg={organization.icon_bg}
												class="size-4" />
										</span>
										<span class="min-w-0">
											<span class="block truncate font-medium">{organization.name}</span>
											<span class="block text-xs text-muted-foreground">{organization.plan}</span>
										</span>
										{#if organization.is_root}
											<span
												class="inline-flex shrink-0 items-center rounded-full border px-2 py-0.5 text-xs font-medium">
												{t('account.settings.system')}
											</span>
										{/if}
									</span>
									<ChevronRightIcon
										class="size-4 shrink-0 text-muted-foreground transition-transform group-hover:translate-x-0.5 motion-reduce:transition-none" />
								</a>
							{/each}
						</div>
					{/if}
					<div>
						<Button href="/organizations/add" size="sm">
							<PlusIcon />
							{t('account.settings.add_organization')}
						</Button>
					</div>
				</SettingsSection>
			{/if}
		</div>
	</div>

	<AlertDialog.Root open={confirmTarget !== null}>
		<AlertDialog.Content>
			<AlertDialog.Header>
				<AlertDialog.Title
					>{t('account.settings.restart_confirm_title', {
						target: confirmTarget,
					})}</AlertDialog.Title>
				<AlertDialog.Description>
					{#if confirmTarget}
						{infrastructureTargets.find((t) => t.key === confirmTarget)?.description}
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
{/if}
