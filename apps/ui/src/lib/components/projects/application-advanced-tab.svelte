<script lang="ts">
	import { Button } from '#lib/components/ui/button/index.js';
	import SoftCheckbox from '#lib/components/custom/soft-checkbox.svelte';
	import SettingsSection from './settings-section.svelte';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import SoftSwitch from '#lib/components/custom/soft-switch.svelte';
	import EyeIcon from '@lucide/svelte/icons/eye';
	import EyeOffIcon from '@lucide/svelte/icons/eye-off';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import InfoIcon from '@lucide/svelte/icons/info';
	import UserIcon from '@lucide/svelte/icons/user';
	import * as Select from '#lib/components/ui/select/index.js';
	import { Slider } from '#lib/components/ui/slider/index.js';
	import { resourceLimitGuidance } from './resource-limit-guidance';
	import { toneBadge, toneDot } from '#lib/utils/status-tone.js';
	import type {
		BasicAuthConfigDto,
		BasicAuthUserDto,
		DomainDto,
		HealthCheckDto,
		SourceConfigDto,
		ServerSummaryDto,
		UpdateHealthCheckInput,
	} from '#lib/clients/project-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import { untrack } from 'svelte';

	let {
		server,
		onLoadResourceBaseline,
		source,
		domains,
		busy = false,
		onSaveSource,
		basicAuth,
		onSaveBasicAuth,
		onAddBasicAuthUser,
		onDeleteBasicAuthUser,
		healthCheck = null,
		onSaveHealthCheck,
	}: {
		server: ServerSummaryDto | null;
		onLoadResourceBaseline: () => Promise<{ cpu_cores: number; memory_bytes: number } | null>;
		source: SourceConfigDto;
		domains: DomainDto[];
		busy?: boolean;
		onSaveSource: (input: SourceConfigDto) => Promise<void>;
		basicAuth: BasicAuthConfigDto;
		onSaveBasicAuth: (input: {
			enabled?: boolean;
			realm?: string;
			protected_domain_ids?: string[];
		}) => Promise<BasicAuthConfigDto>;
		onAddBasicAuthUser: (input: {
			username: string;
			password: string;
		}) => Promise<BasicAuthUserDto>;
		onDeleteBasicAuthUser: (userId: string) => Promise<void>;
		healthCheck?: HealthCheckDto | null;
		onSaveHealthCheck: (input: UpdateHealthCheckInput) => Promise<void>;
	} = $props();

	let resourceEnabled = $state(false);
	let cpuCores = $state('');
	let memoryMib = $state('');
	let resourceBaseline = $state<{ cpu_cores: number; memory_bytes: number } | null>(null);
	let baselineLoading = $state(false);
	const guidance = $derived(
		resourceLimitGuidance(
			server?.capacity ?? { cpu_cores: null, memory_bytes: null, disk_free_bytes: null },
			resourceBaseline,
		),
	);

	$effect(() => {
		resourceEnabled = source.build_cpu_cores !== null && source.build_memory_mib !== null;
		cpuCores = source.build_cpu_cores?.toString() ?? '';
		memoryMib = source.build_memory_mib?.toString() ?? '';
	});

	function toggleResourceLimits(next: boolean) {
		resourceEnabled = next;
		if (next) {
			if (!cpuCores) cpuCores = String(guidance.recommendedCpu ?? 1);
			if (!memoryMib) memoryMib = String(guidance.recommendedMemoryMib ?? 512);
			if (!resourceBaseline && !baselineLoading) {
				baselineLoading = true;
				void onLoadResourceBaseline()
					.then((baseline) => {
						resourceBaseline = baseline;
					})
					.catch(() => {
						resourceBaseline = null;
					})
					.finally(() => {
						baselineLoading = false;
					});
			}
		}
	}

	async function saveResourceLimits() {
		try {
			if (!resourceEnabled) {
				await onSaveSource({ ...source, build_cpu_cores: null, build_memory_mib: null });
				return;
			}
			const cpu = Number(cpuCores);
			const memory = Number(memoryMib);
			if (!Number.isFinite(cpu) || cpu < 0.1) {
				toast.error(t('projects.application_advanced.cpu_limit_invalid'));
				return;
			}
			if (!Number.isInteger(memory) || memory <= 0) {
				toast.error(t('projects.application_advanced.memory_limit_invalid'));
				return;
			}
			if (guidance.maxCpu !== null && cpu > guidance.maxCpu) {
				toast.error(t('projects.application_advanced.cpu_over_capacity'));
				return;
			}
			if (guidance.maxMemoryMib !== null && memory > guidance.maxMemoryMib) {
				toast.error(t('projects.application_advanced.memory_over_capacity'));
				return;
			}
			await onSaveSource({ ...source, build_cpu_cores: cpu, build_memory_mib: memory });
		} catch {
			// The page-level save handler already presents the server error.
		}
	}

	// ── Basic Authentication card ───────────────────────────────────────────

	let realmInput = $state('Restricted');
	let protectedIds = $state<Set<string>>(new Set());
	let addUserOpen = $state(false);
	let savingBasicAuth = $state(false);
	let savingProtected = $state(false);

	$effect(() => {
		realmInput = basicAuth.realm || 'Restricted';
		protectedIds = new Set(basicAuth.protected_domain_ids);
	});

	async function toggleEnabled(next: boolean) {
		if (!next && basicAuth.users.length === 0) {
			toast.error(t('projects.application_advanced.no_users_before_disable'));
			return;
		}
		savingBasicAuth = true;
		try {
			await onSaveBasicAuth({ enabled: next });
		} finally {
			savingBasicAuth = false;
		}
	}

	async function commitRealm() {
		const trimmed = realmInput.trim();
		if (trimmed === '' || trimmed === basicAuth.realm) {
			realmInput = basicAuth.realm || 'Restricted';
			return;
		}
		savingBasicAuth = true;
		try {
			await onSaveBasicAuth({ realm: trimmed });
		} finally {
			savingBasicAuth = false;
		}
	}

	async function toggleDomain(domainId: string, checked: boolean) {
		const next = new Set(protectedIds);
		if (checked) next.add(domainId);
		else next.delete(domainId);
		protectedIds = next;
		savingProtected = true;
		try {
			await onSaveBasicAuth({ protected_domain_ids: [...next] });
		} finally {
			savingProtected = false;
		}
	}

	async function selectAllDomains() {
		const next = new Set(domains.map((d) => d.id));
		protectedIds = next;
		savingProtected = true;
		try {
			await onSaveBasicAuth({ protected_domain_ids: [...next] });
		} finally {
			savingProtected = false;
		}
	}

	async function clearDomainSelection() {
		protectedIds = new Set();
		savingProtected = true;
		try {
			await onSaveBasicAuth({ protected_domain_ids: [] });
		} finally {
			savingProtected = false;
		}
	}

	const allDomainsChecked = $derived(
		domains.length > 0 && domains.every((d) => protectedIds.has(d.id)),
	);

	// ── Add user dialog ─────────────────────────────────────────────────────

	let newUsername = $state('');
	let newPassword = $state('');
	let showPassword = $state(false);
	let savingUser = $state(false);

	const passwordOk = $derived(newPassword.length >= 8 && newPassword.length <= 256);
	const usernameOk = $derived(
		newUsername.trim().length > 0 &&
			newUsername.trim().length <= 64 &&
			/^[A-Za-z0-9._-]+$/.test(newUsername.trim()),
	);
	const canSubmitUser = $derived(usernameOk && passwordOk && !savingUser);

	$effect(() => {
		// Reset form whenever the dialog closes.
		if (!addUserOpen) {
			newUsername = '';
			newPassword = '';
			showPassword = false;
			savingUser = false;
		}
	});

	async function submitAddUser() {
		if (!canSubmitUser) return;
		savingUser = true;
		try {
			await onAddBasicAuthUser({ username: newUsername.trim(), password: newPassword });
			addUserOpen = false;
		} catch {
			// Toast handled by page-level error path.
		} finally {
			savingUser = false;
		}
	}

	async function deleteUser(user: BasicAuthUserDto) {
		const confirmed = confirm(
			t('projects.application_advanced.remove_user_confirm', { name: user.username }),
		);
		if (!confirmed) return;
		try {
			await onDeleteBasicAuthUser(user.id);
		} catch {
			// Toast handled by page-level error path.
		}
	}

	const healthTone = $derived(
		healthCheck?.last_status === 'healthy'
			? 'ok'
			: healthCheck?.last_status === 'unhealthy'
				? 'danger'
				: 'neutral',
	);
	let healthEditorOpen = $state(false);
	let healthEnabled = $state(untrack(() => healthCheck?.enabled ?? false));
	let healthProtocol = $state<'http' | 'tcp'>(untrack(() => healthCheck?.protocol ?? 'http'));
	let healthPath = $state(untrack(() => healthCheck?.path ?? '/'));
	let healthPort = $state(untrack(() => healthCheck?.port?.toString() ?? ''));
	let healthInterval = $state(untrack(() => healthCheck?.interval_seconds ?? 30));
	let healthTimeout = $state(untrack(() => healthCheck?.timeout_seconds ?? 5));
	let healthThreshold = $state(untrack(() => healthCheck?.unhealthy_threshold ?? 3));
	let healthRestart = $state(untrack(() => healthCheck?.restart_on_unhealthy ?? false));
	let healthSaving = $state(false);
	$effect(() => {
		if (healthEditorOpen) return;
		healthEnabled = healthCheck?.enabled ?? false;
		healthProtocol = healthCheck?.protocol ?? 'http';
		healthPath = healthCheck?.path ?? '/';
		healthPort = healthCheck?.port?.toString() ?? '';
		healthInterval = healthCheck?.interval_seconds ?? 30;
		healthTimeout = healthCheck?.timeout_seconds ?? 5;
		healthThreshold = healthCheck?.unhealthy_threshold ?? 3;
		healthRestart = healthCheck?.restart_on_unhealthy ?? false;
	});

	async function submitHealthCheck() {
		healthSaving = true;
		try {
			await onSaveHealthCheck({
				enabled: healthEnabled,
				protocol: healthProtocol,
				path: healthPath,
				port: healthPort.trim() ? Number(healthPort) : null,
				interval_seconds: healthInterval,
				timeout_seconds: healthTimeout,
				unhealthy_threshold: healthThreshold,
				restart_on_unhealthy: healthRestart,
			});
			healthEditorOpen = false;
		} finally {
			healthSaving = false;
		}
	}
</script>

{#snippet capSlider(
	label: string,
	min: number,
	step: number,
	value: number,
	total: number,
	recommended: number | null,
	onChange: (value: number) => void,
)}
	<span>{label}</span>
	<span class="relative block py-1">
		<Slider
			type="single"
			class="resource-slider"
			aria-label={label}
			{min}
			max={total}
			{step}
			value={Math.min(Math.max(value || recommended || min, min), total)}
			disabled={!resourceEnabled || busy}
			onValueChange={(next) => {
				if (next !== undefined) onChange(next);
			}} />
		{#if recommended !== null}
			<b
				aria-hidden="true"
				class="pointer-events-none absolute top-1/2 h-3.5 w-0.5 -translate-y-1/2 rounded-sm bg-warning"
				style:left={`${Math.min(100, ((recommended - min) / (total - min)) * 100)}%`}></b>
		{/if}
	</span>
{/snippet}

<div class="flex max-w-[820px] flex-col gap-4">
	<SettingsSection
		title={t('projects.application_advanced.resources_title')}
		description={t('projects.application_advanced.resources_description')}>
		<form
			class="flex flex-col gap-4"
			onsubmit={(event) => {
				event.preventDefault();
				void saveResourceLimits();
			}}>
			<div class="flex items-center justify-between gap-4">
				<div class="min-w-0">
					<Label for="resource-limits">{t('projects.application_advanced.limit_resources')}</Label>
					<p class="mt-1 text-xs text-muted-foreground">
						{t('projects.application_advanced.limit_resources_hint')}
					</p>
				</div>
				<SoftSwitch
					id="resource-limits"
					checked={resourceEnabled}
					onCheckedChange={(next) => toggleResourceLimits(next === true)}
					disabled={busy} />
			</div>
			<div
				class="flex flex-col gap-3.5 transition-opacity duration-200 motion-reduce:transition-none"
				class:opacity-60={!resourceEnabled}>
				<div class="grid gap-4 sm:grid-cols-2">
					<div class="grid content-start gap-1.5">
						<Label for="resource-cpu-cores">{t('projects.application_advanced.cpu_cores')}</Label>
						<Input
							id="resource-cpu-cores"
							type="number"
							min="0.1"
							max={guidance.maxCpu ?? undefined}
							step="0.1"
							placeholder="1.5"
							bind:value={cpuCores}
							disabled={!resourceEnabled || busy}
							class="font-mono tabular-nums" />
						{#if guidance.maxCpu === null}
							<p class="text-xs text-muted-foreground">
								{t('projects.application_advanced.capacity_unknown')}
							</p>
						{:else if guidance.recommendedCpu !== null}
							<p class="text-xs text-muted-foreground">
								{t('projects.application_advanced.maximum_recommended', {
									value: guidance.recommendedCpu,
									unit: t('projects.application_advanced.cores_unit'),
								})}
							</p>
						{/if}
					</div>
					<div class="grid content-start gap-1.5">
						<Label for="resource-memory-mib">{t('projects.application_advanced.memory_mib')}</Label>
						<Input
							id="resource-memory-mib"
							type="number"
							min="1"
							max={guidance.maxMemoryMib ?? undefined}
							step="1"
							placeholder="2048"
							bind:value={memoryMib}
							disabled={!resourceEnabled || busy}
							class="font-mono tabular-nums" />
						{#if guidance.maxMemoryMib === null}
							<p class="text-xs text-muted-foreground">
								{t('projects.application_advanced.capacity_unknown')}
							</p>
						{:else if guidance.recommendedMemoryMib !== null}
							<p class="text-xs text-muted-foreground">
								{t('projects.application_advanced.maximum_recommended', {
									value: guidance.recommendedMemoryMib,
									unit: 'MiB',
								})}
							</p>
						{/if}
					</div>
				</div>
				{#if guidance.maxCpu !== null || guidance.maxMemoryMib !== null}
					<div class="flex flex-col gap-1.5 text-xs text-muted-foreground">
						{#if guidance.maxCpu !== null && guidance.maxCpu > 0.1}
							{@render capSlider(
								t('projects.application_advanced.cpu_capacity_bar', {
									server: server?.name ?? '',
									total: guidance.maxCpu,
								}),
								0.1,
								0.1,
								Number(cpuCores) || 0,
								guidance.maxCpu,
								guidance.recommendedCpu,
								(next) => (cpuCores = String(Math.round(next * 10) / 10)),
							)}
						{/if}
						{#if guidance.maxMemoryMib !== null && guidance.maxMemoryMib > 64}
							{@render capSlider(
								t('projects.application_advanced.memory_capacity_bar', {
									total: guidance.maxMemoryMib,
								}),
								64,
								64,
								Number(memoryMib) || 0,
								guidance.maxMemoryMib,
								guidance.recommendedMemoryMib,
								(next) => (memoryMib = String(next)),
							)}
						{/if}
					</div>
				{/if}
				{#if server}
					<p class="flex items-start gap-2 text-xs text-muted-foreground">
						<InfoIcon class="mt-0.5 size-3.5 shrink-0" />
						<span
							>{baselineLoading
								? t('projects.application_advanced.server_capacity_sampling', {
										server: server.name,
									})
								: resourceBaseline
									? t('projects.application_advanced.server_capacity_sampled', {
											server: server.name,
										})
									: t('projects.application_advanced.server_capacity_unavailable', {
											server: server.name,
										})}</span>
					</p>
				{/if}
				{#if source.build_method === 'compose'}
					<p
						class="flex items-start gap-2 rounded-[10px] border border-info/25 bg-info/8 px-3 py-2 text-xs">
						<InfoIcon class="mt-0.5 size-3.5 shrink-0 text-info" />
						{t('projects.application_advanced.compose_cap_note')}
					</p>
				{/if}
				{#if resourceEnabled && guidance.maxCpu !== null && guidance.maxMemoryMib !== null && Number(cpuCores) >= guidance.maxCpu && Number(memoryMib) >= guidance.maxMemoryMib}
					<p class="flex items-start gap-2 text-xs text-muted-foreground">
						<InfoIcon class="mt-0.5 size-3.5 shrink-0" />{t(
							'projects.application_advanced.disable_limits_suggestion',
						)}
					</p>
				{/if}
			</div>
			<div class="flex justify-end">
				<Button type="submit" disabled={busy}>
					{busy
						? t('projects.projects.saving')
						: t('projects.application_advanced.save_resource_limits')}
				</Button>
			</div>
		</form>
	</SettingsSection>

	<SettingsSection
		title={t('projects.application_advanced.basicauth_title')}
		description={t('projects.application_advanced.basicauth_description')}>
		<div class="flex items-center justify-between gap-4">
			<div class="min-w-0">
				<Label for="basicauth-enabled">{t('projects.application_advanced.enable_basicauth')}</Label>
				<p class="mt-1 text-xs text-muted-foreground">
					{t('projects.application_advanced.basicauth_disabled_hint')}
				</p>
			</div>
			<SoftSwitch
				id="basicauth-enabled"
				checked={basicAuth.enabled}
				disabled={savingBasicAuth || (!basicAuth.enabled && basicAuth.users.length === 0)}
				onCheckedChange={(next) => void toggleEnabled(next === true)} />
		</div>

		<div class="grid gap-1.5">
			<Label for="basicauth-realm">{t('projects.application_advanced.realm')}</Label>
			<Input
				id="basicauth-realm"
				type="text"
				maxlength={64}
				bind:value={realmInput}
				onblur={() => void commitRealm()}
				disabled={savingBasicAuth}
				placeholder={t('projects.application_advanced.realm_placeholder')} />
			<p class="text-xs text-muted-foreground">
				{t('projects.application_advanced.realm_hint')}
			</p>
		</div>

		<div class="lp-groove-t grid gap-2.5 pt-4">
			<div class="flex flex-wrap items-center justify-between gap-2">
				<div>
					<p class="text-[12.5px] font-medium">{t('projects.application_advanced.users')}</p>
					<p class="text-xs text-muted-foreground">
						{#if basicAuth.users.length === 0}
							{t('projects.application_advanced.no_users')}
						{:else}
							{basicAuth.users.length === 1
								? t('projects.application_advanced.users_configured_one', {
										count: basicAuth.users.length,
									})
								: t('projects.application_advanced.users_configured_many', {
										count: basicAuth.users.length,
									})}
						{/if}
					</p>
				</div>
				<Button
					type="button"
					variant="outline"
					size="sm"
					onclick={() => (addUserOpen = true)}
					disabled={savingUser}>
					<PlusIcon class="size-4" />
					{t('projects.application_advanced.add_user')}
				</Button>
			</div>

			{#if basicAuth.users.length > 0}
				<div class="flex flex-col gap-1.5">
					{#each basicAuth.users as user (user.id)}
						<div
							class="lp-well grid grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-2.5 rounded-[10px] bg-muted px-2.5 py-2">
							<span
								class="lp-raised grid size-7 place-items-center rounded-full bg-card text-muted-foreground"
								><UserIcon class="size-3.5" /></span>
							<div class="grid min-w-0 gap-0.5">
								<span class="truncate text-[13px] font-medium">{user.username}</span>
								<span class="text-xs text-muted-foreground">
									{user.has_password
										? t('projects.application_advanced.password_set')
										: t('projects.application_advanced.no_password_set')}
								</span>
							</div>
							<Button
								type="button"
								variant="ghost"
								size="icon"
								aria-label={t('projects.application_advanced.remove_user', {
									name: user.username,
								})}
								onclick={() => void deleteUser(user)}>
								<Trash2Icon class="size-4 text-muted-foreground" />
							</Button>
						</div>
					{/each}
				</div>
			{/if}
		</div>

		<div class="lp-groove-t grid gap-2.5 pt-4">
			<div class="flex flex-wrap items-center justify-between gap-2">
				<div>
					<p class="text-[12.5px] font-medium">
						{t('projects.application_advanced.protected_domains_title')}
					</p>
					<p class="text-xs text-muted-foreground">
						{#if domains.length === 0}
							{t('projects.application_advanced.no_domains')}
						{:else}
							{domains.length === 1
								? t('projects.application_advanced.protected_count_one', {
										protected: protectedIds.size,
										total: domains.length,
									})
								: t('projects.application_advanced.protected_count_many', {
										protected: protectedIds.size,
										total: domains.length,
									})}
						{/if}
					</p>
				</div>
				{#if domains.length > 0}
					<div class="flex gap-2">
						<Button
							type="button"
							variant="outline"
							size="sm"
							onclick={() => void selectAllDomains()}
							disabled={savingProtected || !basicAuth.enabled || allDomainsChecked}>
							{t('projects.environment.select_all')}
						</Button>
						<Button
							type="button"
							variant="outline"
							size="sm"
							onclick={() => void clearDomainSelection()}
							disabled={savingProtected || !basicAuth.enabled || protectedIds.size === 0}>
							{t('projects.common.clear')}
						</Button>
					</div>
				{/if}
			</div>
			{#if domains.length > 0}
				<p class="text-xs text-muted-foreground">
					{t('projects.application_advanced.protected_domains_description')}
				</p>
				<div class="flex flex-col gap-1.5">
					{#each domains as domain (domain.id)}
						{@const isProtected = protectedIds.has(domain.id)}
						<label
							class="flex items-start gap-3 rounded-[10px] border border-foreground/9 px-3 py-2.5 transition-colors hover:bg-muted/40 {basicAuth.enabled
								? 'cursor-pointer'
								: 'cursor-not-allowed opacity-60'}">
							<SoftCheckbox
								id={`basicauth-domain-${domain.id}`}
								checked={isProtected}
								disabled={!basicAuth.enabled || savingProtected}
								onCheckedChange={(next) => void toggleDomain(domain.id, next === true)}
								class="mt-0.5" />
							<div class="grid min-w-0 gap-0.5">
								<span class="flex flex-wrap items-center gap-1.5 text-[13px] font-medium">
									<span class="truncate font-mono">{domain.host}</span>
									{#if domain.is_primary}
										<span
											class="rounded-full bg-primary/10 px-1.5 py-px font-mono text-[10.5px] text-primary">
											{t('projects.dialogs_common.primary')}
										</span>
									{/if}
									{#if domain.https}
										<span
											class="rounded-full bg-muted px-1.5 py-px font-mono text-[10.5px] text-muted-foreground">
											HTTPS
										</span>
									{/if}
								</span>
								<span class="truncate text-xs text-muted-foreground">
									{domain.is_local
										? t('projects.application_advanced.local_port', { port: domain.port })
										: domain.url}
								</span>
							</div>
						</label>
					{/each}
				</div>
				{#if !basicAuth.enabled}
					<p class="text-xs text-muted-foreground">
						{t('projects.application_advanced.basicauth_disabled_note')}
					</p>
				{/if}
			{/if}
		</div>
	</SettingsSection>

	<SettingsSection
		title={t('projects.health_check.title')}
		description={healthCheck?.enabled ? undefined : t('projects.health_check.disabled_hint')}>
		{#snippet aside()}
			<div class="mt-2.5">
				{#if healthCheck?.enabled}
					<span
						class="inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-xs font-medium {toneBadge(
							healthTone,
						)}">
						<span class="size-1.5 rounded-full {toneDot(healthTone)}"></span>
						{healthCheck.last_status === 'healthy'
							? t('projects.health_check.healthy')
							: healthCheck.last_status === 'unhealthy'
								? t('projects.health_check.unhealthy')
								: t('projects.health_check.unknown')}
					</span>
				{:else}
					<span class="rounded-full bg-muted px-2 py-0.5 text-xs font-medium text-muted-foreground">
						{t('projects.health_check.disabled')}
					</span>
				{/if}
			</div>
		{/snippet}
		{#if !healthEditorOpen}
			<div class="flex flex-wrap items-center justify-between gap-3">
				<p class="min-w-0 text-[13px] text-muted-foreground">
					{#if healthCheck?.enabled}
						{t('projects.health_check.summary', {
							protocol: healthCheck.protocol.toUpperCase(),
							target:
								healthCheck.protocol === 'http'
									? healthCheck.path
									: `:${healthCheck.port ?? t('projects.health_check.auto_port')}`,
							interval: healthCheck.interval_seconds,
						})}
						{#if healthCheck.consecutive_failures > 0}
							· {t('projects.health_check.consecutive_failures', {
								count: healthCheck.consecutive_failures,
							})}
						{/if}
					{/if}
				</p>
				<Button variant="outline" size="sm" onclick={() => (healthEditorOpen = true)}>
					{t('projects.health_check.configure')}
				</Button>
			</div>
		{:else}
			<label class="flex items-center gap-2 text-sm"
				><SoftSwitch bind:checked={healthEnabled} />{t('projects.health_check.enable')}</label>
			{#if healthEnabled}
				<div class="grid gap-4 sm:grid-cols-2">
					<div class="grid gap-1.5">
						<Label>{t('projects.health_check.protocol')}</Label>
						<Select.Root type="single" bind:value={healthProtocol}>
							<Select.Trigger class="w-full"
								>{healthProtocol === 'http'
									? t('projects.health_check.protocol_http')
									: t('projects.health_check.protocol_tcp')}</Select.Trigger>
							<Select.Content>
								<Select.Item value="http">{t('projects.health_check.protocol_http')}</Select.Item>
								<Select.Item value="tcp">{t('projects.health_check.protocol_tcp')}</Select.Item>
							</Select.Content>
						</Select.Root>
					</div>
					<div class="grid gap-1.5">
						<Label>{t('projects.health_check.port')}</Label>
						<Input
							bind:value={healthPort}
							placeholder={t('projects.health_check.auto_port')}
							inputmode="numeric"
							class="font-mono" />
					</div>
					{#if healthProtocol === 'http'}
						<div class="grid gap-1.5 sm:col-span-2">
							<Label>{t('projects.health_check.path')}</Label>
							<Input bind:value={healthPath} placeholder="/" class="font-mono" />
						</div>
					{/if}
					<div class="grid gap-1.5">
						<Label>{t('projects.health_check.interval')}</Label>
						<Input type="number" min="5" max="3600" bind:value={healthInterval} />
					</div>
					<div class="grid gap-1.5">
						<Label>{t('projects.health_check.timeout')}</Label>
						<Input type="number" min="1" max="60" bind:value={healthTimeout} />
					</div>
					<div class="grid gap-1.5">
						<Label>{t('projects.health_check.threshold')}</Label>
						<Input type="number" min="1" max="20" bind:value={healthThreshold} />
					</div>
				</div>
				<label class="flex items-center gap-2 text-sm"
					><SoftSwitch bind:checked={healthRestart} />{t(
						'projects.health_check.restart_on_unhealthy',
					)}</label>
				<p class="text-xs text-muted-foreground">
					{t('projects.health_check.restart_hint')}
				</p>
			{/if}
			<div class="flex justify-end gap-2">
				<Button variant="outline" onclick={() => (healthEditorOpen = false)}
					>{t('projects.health_check.cancel')}</Button>
				<Button disabled={healthSaving} onclick={submitHealthCheck}
					>{healthSaving
						? t('projects.health_check.saving')
						: t('projects.health_check.save')}</Button>
			</div>
		{/if}
	</SettingsSection>
</div>

<Dialog.Root bind:open={addUserOpen}>
	<Dialog.Content>
		<Dialog.Header>
			<Dialog.Title>{t('projects.application_advanced.add_user_title')}</Dialog.Title>
			<Dialog.Description>
				{t('projects.application_advanced.add_user_description')}
			</Dialog.Description>
		</Dialog.Header>
		<form
			class="grid gap-4"
			onsubmit={(event) => {
				event.preventDefault();
				void submitAddUser();
			}}>
			<div class="grid gap-2">
				<Label for="basicauth-username">{t('projects.application_advanced.username')}</Label>
				<Input
					id="basicauth-username"
					type="text"
					bind:value={newUsername}
					placeholder={t('projects.application_advanced.username_placeholder')}
					maxlength={64}
					autocomplete="off"
					required />
				{#if newUsername.trim().length > 0 && !usernameOk}
					<p class="text-xs text-destructive">
						{t('projects.application_advanced.username_hint')}
					</p>
				{/if}
			</div>
			<div class="grid gap-2">
				<Label for="basicauth-password">{t('projects.application_advanced.password')}</Label>
				<div class="relative">
					<Input
						id="basicauth-password"
						type={showPassword ? 'text' : 'password'}
						bind:value={newPassword}
						placeholder={t('projects.application_advanced.password_placeholder')}
						minlength={8}
						maxlength={256}
						autocomplete="new-password"
						class="pr-10"
						required />
					<button
						type="button"
						class="absolute inset-y-0 right-0 flex items-center px-3 text-muted-foreground hover:text-foreground"
						aria-label={showPassword
							? t('projects.application_advanced.hide_password')
							: t('projects.application_advanced.show_password')}
						onclick={() => (showPassword = !showPassword)}>
						{#if showPassword}
							<EyeOffIcon class="size-4" />
						{:else}
							<EyeIcon class="size-4" />
						{/if}
					</button>
				</div>
				{#if newPassword.length > 0 && !passwordOk}
					<p class="text-xs text-destructive">
						{t('projects.application_advanced.password_too_short')}
					</p>
				{/if}
			</div>
			<Dialog.Footer>
				<Button
					type="button"
					variant="outline"
					onclick={() => (addUserOpen = false)}
					disabled={savingUser}>
					{t('projects.projects.cancel')}
				</Button>
				<Button type="submit" disabled={!canSubmitUser}>
					{savingUser
						? t('projects.application_advanced.adding')
						: t('projects.application_advanced.add_user')}
				</Button>
			</Dialog.Footer>
		</form>
	</Dialog.Content>
</Dialog.Root>

<style>
	/* Recessed well track, accent fill, glossy thumb (same recipe as soft-switch). */
	:global(.resource-slider [data-slot='slider-track']) {
		height: 0.5rem;
		background: linear-gradient(
			180deg,
			color-mix(in oklch, var(--lp-deep) 88%, black),
			var(--lp-deep) 70%
		);
		box-shadow:
			inset 0 1px 2px rgb(0 0 0 / 0.22),
			inset 0 -1px 1px var(--lp-bevel),
			0 0 0 1px color-mix(in oklch, var(--lp-ink) 8%, transparent);
	}
	:global(.dark .resource-slider [data-slot='slider-track']) {
		background: linear-gradient(180deg, #05060a, color-mix(in oklch, var(--lp-deep) 60%, black));
		box-shadow:
			inset 0 1px 3px rgb(0 0 0 / 0.8),
			0 0 0 1px rgb(0 0 0 / 0.6);
	}
	:global(.resource-slider [data-slot='slider-range']) {
		background: linear-gradient(
			180deg,
			color-mix(in oklch, var(--primary) 72%, white),
			var(--primary) 60%,
			color-mix(in oklch, var(--primary) 82%, black)
		);
		box-shadow: inset 0 1px 1px rgb(255 255 255 / 0.3);
	}
	:global(.resource-slider [data-slot='slider-thumb']) {
		width: 1.125rem;
		height: 1.125rem;
		border-radius: 999px;
		background:
			radial-gradient(60% 55% at 35% 28%, rgb(255 255 255 / 0.95), rgb(255 255 255 / 0) 70%),
			linear-gradient(180deg, #fdfdfe, #e9ebf0 70%, #dfe2e8);
		box-shadow:
			inset 0 -2px 3px rgb(16 22 40 / 0.14),
			inset 0 1px 1px #fff,
			0 0 0 0.5px rgb(16 22 40 / 0.12),
			0 2px 3px rgb(16 22 40 / 0.22),
			0 6px 10px -3px rgb(16 22 40 / 0.28);
	}
	:global(.dark .resource-slider [data-slot='slider-thumb']) {
		background:
			radial-gradient(60% 55% at 35% 28%, rgb(255 255 255 / 0.7), rgb(255 255 255 / 0) 70%),
			linear-gradient(180deg, #e6e9ef, #b9bec8);
		box-shadow:
			inset 0 -2px 3px rgb(0 0 0 / 0.25),
			inset 0 1px 1px rgb(255 255 255 / 0.8),
			0 2px 3px rgb(0 0 0 / 0.5);
	}
</style>
