<script lang="ts">
	import CheckIcon from '@lucide/svelte/icons/check';
	import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import CopyButton from '#lib/components/custom/copy-button.svelte';
	import {
		applyServerSecurity,
		auditServerSecurity,
		type SecurityAction,
		type Server,
		type ServerMaintenanceResult,
		type ServerSecurity,
	} from '#lib/clients/server-client.js';
	import {
		firewallCommands,
		securityStepStatus,
		sshManualCommands,
		type SecurityStep,
	} from '#lib/utils/server-security.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		open = $bindable(false),
		organizationId,
		server,
	}: {
		open?: boolean;
		organizationId: string;
		server: Server | null;
	} = $props();

	const STEPS: SecurityStep[] = ['ssh', 'fail2ban', 'firewall', 'updates'];

	let report = $state<ServerSecurity | null>(null);
	let loadError = $state('');
	let loading = $state(false);
	let running = $state<SecurityAction | null>(null);
	let results = $state<Partial<Record<SecurityStep, ServerMaintenanceResult>>>({});
	let sshLevel = $state<'recommended' | 'strict'>('recommended');
	let manual = $state<Partial<Record<SecurityStep, boolean>>>({});
	let sshConfirmOpen = $state(false);

	const status = $derived(report ? securityStepStatus(report) : null);
	const doneCount = $derived(status ? STEPS.filter((step) => status[step] === 'done').length : 0);
	const connectsAsRoot = $derived(server?.ssh_user === 'root');
	const firewall = $derived(
		report
			? firewallCommands(
					report.firewall,
					server?.ssh_port ?? null,
					server?.ingress_mode === 'public_edge',
				).join('\n')
			: '',
	);

	async function load() {
		if (!server) return;
		loading = true;
		loadError = '';
		try {
			report = await auditServerSecurity(organizationId, server.id);
			if (report.ssh_level) sshLevel = report.ssh_level;
		} catch (caught) {
			loadError = caught instanceof Error ? caught.message : t('servers.security.load_failed');
		} finally {
			loading = false;
		}
	}

	async function apply(step: SecurityStep, action: SecurityAction) {
		if (!server || running) return;
		running = action;
		sshConfirmOpen = false;
		try {
			const result = await applyServerSecurity(organizationId, server.id, action);
			results[step] = result;
			if (result.success) {
				toast.success(t('servers.security.applied_toast', { name: server.name }));
				await load();
			} else {
				toast.error(t('servers.security.apply_failed', { name: server.name }));
			}
		} catch (caught) {
			toast.error(
				caught instanceof Error
					? caught.message
					: t('servers.security.apply_failed', { name: server.name }),
			);
		} finally {
			running = null;
		}
	}

	$effect(() => {
		if (open) {
			report = null;
			results = {};
			manual = {};
			running = null;
			sshLevel = 'recommended';
			void load();
		}
	});
</script>

{#snippet commandBlock(value: string)}
	<div class="flex items-start gap-2 rounded-md border bg-muted/50 p-2 font-mono text-xs">
		<code class="min-w-0 flex-1 break-all whitespace-pre-wrap">{value}</code>
		<CopyButton
			{value}
			label={null}
			revealOnHover={false}
			class="mt-0.5 shrink-0 text-muted-foreground hover:text-foreground" />
	</div>
{/snippet}

{#snippet resultBlock(step: SecurityStep)}
	{@const result = results[step]}
	{#if result && (result.stdout.trim() || result.stderr.trim())}
		<pre
			class="max-h-40 overflow-auto rounded-md border p-2 text-xs whitespace-pre-wrap {result.success
				? 'bg-muted/40'
				: 'border-destructive/30 bg-destructive/5'}">{(result.success
				? result.stdout
				: result.stderr || result.stdout
			).trim()}</pre>
	{/if}
{/snippet}

{#snippet applyButton(
	step: SecurityStep,
	action: SecurityAction,
	label: string,
	onclick?: () => void,
)}
	<Button
		size="sm"
		disabled={running !== null}
		onclick={onclick ?? (() => void apply(step, action))}>
		{#if running === action}<LoaderCircleIcon class="size-4 animate-spin" />{/if}
		{running === action ? t('servers.security.applying') : label}
	</Button>
{/snippet}

{#snippet manualToggle(step: SecurityStep)}
	<Button size="sm" variant="outline" onclick={() => (manual[step] = !manual[step])}>
		{t('servers.security.manual')}
	</Button>
{/snippet}

<Dialog.Root bind:open>
	<Dialog.Content class="max-h-[90vh] overflow-y-auto sm:max-w-2xl">
		<Dialog.Header>
			<Dialog.Title>{t('servers.security.title', { name: server?.name ?? '' })}</Dialog.Title>
			<Dialog.Description>{t('servers.security.description')}</Dialog.Description>
		</Dialog.Header>

		<div class="flex items-center justify-between gap-3">
			<p class="text-sm text-muted-foreground">
				{#if status}
					{t('servers.security.progress', { done: doneCount, total: STEPS.length })}
				{:else if loading}
					{t('servers.security.checking')}
				{/if}
			</p>
			<Button
				size="sm"
				variant="ghost"
				disabled={loading || running !== null}
				onclick={() => void load()}>
				<RefreshCwIcon class="size-4 {loading ? 'animate-spin' : ''}" />
				{t('servers.security.recheck')}
			</Button>
		</div>

		{#if loadError}
			<p
				class="rounded-md border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive">
				{loadError}
			</p>
		{:else if report && status}
			<ol class="space-y-3">
				{#each STEPS as step, index (step)}
					<li class="rounded-lg border p-3">
						<div class="flex items-start gap-3">
							<span
								class="flex size-6 shrink-0 items-center justify-center rounded-full border text-xs font-medium {status[
									step
								] === 'done'
									? 'border-success/40 bg-success/10 text-success'
									: 'text-muted-foreground'}">
								{#if status[step] === 'done'}<CheckIcon class="size-3.5" />{:else}{index + 1}{/if}
							</span>
							<div class="min-w-0 flex-1 space-y-2">
								<div class="flex flex-wrap items-center gap-2">
									<p class="text-sm font-medium">{t(`servers.security.${step}_title`)}</p>
									<Badge variant={status[step] === 'done' ? 'secondary' : 'outline'}>
										{t(`servers.security.status_${status[step]}`)}
									</Badge>
								</div>
								<p class="text-xs text-muted-foreground">{t(`servers.security.${step}_why`)}</p>

								{#if step === 'ssh'}
									<p class="text-xs">
										{report.password_login === null
											? t('servers.security.ssh_unknown')
											: report.password_login
												? t('servers.security.ssh_passwords_on')
												: t('servers.security.ssh_passwords_off')}
										{#if report.root_login}
											· {t('servers.security.ssh_root', { value: report.root_login })}
										{/if}
									</p>
									<div class="grid gap-2 sm:grid-cols-2" role="radiogroup">
										{#each ['recommended', 'strict'] as const as level (level)}
											{@const disabled = level === 'strict' && connectsAsRoot}
											<button
												type="button"
												role="radio"
												aria-checked={sshLevel === level}
												{disabled}
												class="rounded-md border p-2 text-left text-xs transition-colors disabled:cursor-not-allowed disabled:opacity-50 {sshLevel ===
												level
													? 'border-primary bg-primary/5'
													: 'hover:bg-muted/50'}"
												onclick={() => (sshLevel = level)}>
												<span class="flex items-center gap-1.5 font-medium">
													{t(`servers.security.ssh_${level}`)}
													{#if report.ssh_level === level}
														<Badge variant="secondary">{t('servers.security.current')}</Badge>
													{/if}
												</span>
												<span class="mt-1 block text-muted-foreground">
													{disabled
														? t('servers.security.ssh_strict_root')
														: t(`servers.security.ssh_${level}_desc`)}
												</span>
											</button>
										{/each}
									</div>
									<div class="flex flex-wrap gap-2">
										{@render applyButton(
											'ssh',
											sshLevel === 'strict' ? 'ssh_strict' : 'ssh_recommended',
											t('servers.security.apply'),
											() => (sshConfirmOpen = true),
										)}
										{@render manualToggle('ssh')}
									</div>
									{#if manual.ssh && report.ssh_settings}
										<p class="text-xs text-muted-foreground">
											{t('servers.security.ssh_manual_hint')}
										</p>
										{@render commandBlock(sshManualCommands(report.ssh_settings[sshLevel]))}
									{/if}
								{:else if step === 'fail2ban'}
									<p class="text-xs">
										{t(`servers.security.fail2ban_${report.fail2ban}`)}
										{#if report.banned_ips !== null}
											· {t('servers.security.fail2ban_banned', { count: report.banned_ips })}
										{/if}
									</p>
									{#if status.fail2ban !== 'done'}
										{@render applyButton(
											'fail2ban',
											'fail2ban',
											t('servers.security.fail2ban_apply'),
										)}
									{/if}
								{:else if step === 'firewall'}
									<p class="text-xs">
										{report.firewall
											? t(
													report.firewall_active
														? 'servers.security.firewall_active'
														: 'servers.security.firewall_inactive',
													{ tool: report.firewall },
												)
											: t('servers.security.firewall_none')}
									</p>
									{#if report.public_ports.length}
										<div class="flex flex-wrap items-center gap-1 text-xs">
											<span class="text-muted-foreground"
												>{t('servers.security.public_ports')}</span>
											{#each report.public_ports as port (port)}
												<Badge variant="outline" class="font-mono">{port}</Badge>
											{/each}
										</div>
									{/if}
									<div class="space-y-1 rounded-md border p-2 text-xs">
										<p class="font-medium">{t('servers.security.firewall_option_provider')}</p>
										<p class="text-muted-foreground">
											{t('servers.security.firewall_option_provider_desc')}
										</p>
										<ul class="list-disc space-y-0.5 pl-4 text-muted-foreground">
											<li>
												{t('servers.connect.fw_ssh', { port: String(server?.ssh_port ?? 22) })}
											</li>
											{#if server?.ingress_mode === 'public_edge'}
												<li>{t('servers.connect.fw_tls')}</li>
											{/if}
										</ul>
									</div>
									<div class="space-y-2 rounded-md border p-2 text-xs">
										<p class="font-medium">{t('servers.security.firewall_option_host')}</p>
										<p class="text-muted-foreground">
											{t('servers.security.firewall_option_host_desc')}
										</p>
										{@render commandBlock(firewall)}
									</div>
									{#if server?.podman_mode === 'rootful'}
										<p class="flex items-start gap-1.5 text-xs text-warning">
											<TriangleAlertIcon class="mt-0.5 size-3.5 shrink-0" />
											{t('servers.security.firewall_rootful')}
										</p>
									{/if}
								{:else}
									<p class="text-xs">
										{report.auto_updates === null
											? t('servers.security.updates_unsupported')
											: report.auto_updates
												? t('servers.security.updates_on')
												: t('servers.security.updates_off')}
										{#if report.security_updates}
											· {t('servers.security.updates_pending', { count: report.security_updates })}
										{/if}
									</p>
									{#if report.reboot_required}
										<p class="flex items-start gap-1.5 text-xs text-warning">
											<TriangleAlertIcon class="mt-0.5 size-3.5 shrink-0" />
											{t('servers.security.reboot_required')}
										</p>
									{/if}
									{#if report.auto_updates === false}
										<div class="flex flex-wrap gap-2">
											{@render applyButton(
												'updates',
												'auto_updates',
												t('servers.security.updates_apply'),
											)}
											{@render manualToggle('updates')}
										</div>
										{#if manual.updates}
											{@render commandBlock(
												'sudo apt-get install -y unattended-upgrades\nsudo dpkg-reconfigure -plow unattended-upgrades',
											)}
										{/if}
									{/if}
								{/if}
								{@render resultBlock(step)}
							</div>
						</div>
					</li>
				{/each}
			</ol>
		{:else if loading}
			<div class="flex justify-center py-10">
				<LoaderCircleIcon class="size-5 animate-spin text-muted-foreground" />
			</div>
		{/if}

		<Dialog.Footer>
			<Button variant="outline" disabled={running !== null} onclick={() => (open = false)}
				>{t('organizations.common.close')}</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>

<AlertDialog.Root bind:open={sshConfirmOpen}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>
				{t('servers.security.ssh_confirm_title', { level: t(`servers.security.ssh_${sshLevel}`) })}
			</AlertDialog.Title>
			<AlertDialog.Description class="space-y-2">
				<span class="block">{t('servers.security.ssh_confirm_passwords')}</span>
				{#if sshLevel === 'strict'}
					<span class="block">{t('servers.security.ssh_confirm_root')}</span>
				{/if}
				<span class="block">{t('servers.security.ssh_confirm_safety')}</span>
			</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel>{t('organizations.common.cancel')}</AlertDialog.Cancel>
			<Button
				disabled={running !== null}
				onclick={() => void apply('ssh', sshLevel === 'strict' ? 'ssh_strict' : 'ssh_recommended')}>
				{t('servers.security.apply')}
			</Button>
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
