<script lang="ts">
	import { allowedHere } from '#lib/state/permissions.svelte.js';
	import type { DomainInput } from '#lib/clients/project-client.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import CircleHelpIcon from '@lucide/svelte/icons/circle-help';
	import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import ShieldCheckIcon from '@lucide/svelte/icons/shield-check';
	import CheckIcon from '@lucide/svelte/icons/check';
	import ServerIcon from '@lucide/svelte/icons/server';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import LockIcon from '@lucide/svelte/icons/lock';
	import LockOpenIcon from '@lucide/svelte/icons/lock-open';
	import PencilIcon from '@lucide/svelte/icons/pencil';
	import DomainDnsHelpDialog from './domain-dns-help-dialog.svelte';
	import DomainFormDialog from './domain-form-dialog.svelte';
	import type {
		DomainDnsValidationResult,
		DomainDto,
		DomainMarkersDto,
		PortConstraintsDto,
		ServerSummaryDto,
	} from '#lib/clients/project-client.js';
	import type { Server } from '#lib/clients/server-client.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		domains = [],
		server,
		busy = false,
		refreshing = false,
		configStale = false,
		constraints = { reserved_ports: [], taken_ports: [] },
		pinServers = [],
		onCreate,
		onUpdate,
		onDelete,
		onGenerate,
		onValidateDns,
		onValidateHostDns,
		onRefresh,
		onDeploy,
		onRandomSslipHost,
		composeServices = [],
		composeRoutes = [],
		domainMarkers = null,
		onSaveComposeRoute,
	}: {
		domains?: DomainDto[];
		server:
			| (Pick<Server, 'kind' | 'ingress_mode' | 'name'> & { public_address?: string | null })
			| null;
		busy?: boolean;
		refreshing?: boolean;
		configStale?: boolean;
		constraints?: PortConstraintsDto;
		/** Members of a group-targeted application; non-empty when domains must be pinned. */
		pinServers?: ServerSummaryDto[];
		onCreate: (input: DomainInput) => Promise<DomainDto | void>;
		onUpdate: (domainId: string, input: DomainInput) => Promise<DomainDto | void>;
		onDelete: (domainId: string) => Promise<void>;
		onGenerate: () => Promise<void>;
		onValidateDns: (domainId: string) => Promise<DomainDnsValidationResult>;
		/** Checks a hostname typed in the dialog, before it is saved. */
		onValidateHostDns?: (
			host: string,
			serverId: string | null,
		) => Promise<DomainDnsValidationResult>;
		onRefresh: () => void | Promise<void>;
		onDeploy: () => Promise<void>;
		onRandomSslipHost?: () => Promise<string>;
		composeServices?: { name: string; suggested_internal_ports: number[] }[];
		composeRoutes?: { domain_id: string; service: string; container_port: number }[];
		/** `{{url}}`-style env markers and the routes they read. */
		domainMarkers?: DomainMarkersDto | null;
		onSaveComposeRoute?: (
			domainId: string,
			service: string,
			containerPort: number,
		) => Promise<void>;
	} = $props();

	let formOpen = $state(false);
	let dnsHelpOpen = $state(false);
	let editing = $state<DomainDto | null>(null);
	type DnsCheckState = {
		loading: boolean;
		result?: DomainDnsValidationResult;
		requestError?: string;
	};
	let dnsChecks = $state<Record<string, DnsCheckState>>({});

	/** Env variables reading each `service:port` route through a domain marker. */
	const markerVariablesByRoute = $derived.by(() => {
		const byRoute = new Map<string, string[]>();
		for (const marker of domainMarkers?.markers ?? []) {
			if (!marker.service || marker.port === null) continue;
			const key = `${marker.service}:${marker.port}`;
			const names = byRoute.get(key) ?? [];
			if (!names.includes(marker.name)) byRoute.set(key, [...names, marker.name]);
		}
		return byRoute;
	});

	function markerVariables(domainId: string): string[] {
		const route = composeRoutes.find((candidate) => candidate.domain_id === domainId);
		if (!route) return [];
		return markerVariablesByRoute.get(`${route.service}:${route.container_port}`) ?? [];
	}

	/** Variables left without any domain if `domainId` is deleted. */
	function orphanedByDelete(domainId: string): string[] {
		const route = composeRoutes.find((candidate) => candidate.domain_id === domainId);
		if (!route) return [];
		const siblings = composeRoutes.filter(
			(candidate) =>
				candidate.service === route.service && candidate.container_port === route.container_port,
		);
		return siblings.length > 1 ? [] : markerVariables(domainId);
	}

	function add() {
		editing = null;
		formOpen = true;
	}
	function edit(domain: DomainDto) {
		editing = domain;
		formOpen = true;
	}
	function certificateLabel(domain: DomainDto): string {
		if (!domain.https) return 'HTTP';
		if (domain.certificate_provider === 'custom')
			return domain.certificate_name || t('account.certificates.custom');
		if (domain.certificate_provider === 'letsencrypt') return "Let's Encrypt";
		return t('account.certificates.automatic');
	}
	function canValidateDns(domain: DomainDto) {
		return (
			!domain.is_local && !domain.host.endsWith('.sslip.io') && Boolean(domain.required_dns_target)
		);
	}
	/** A group app's domain names its pinned server; a single-server app's routes to the app's server. */
	function serverNameOf(domain: DomainDto): string | null {
		if (domain.server_id) {
			return pinServers.find((pin) => pin.id === domain.server_id)?.name ?? domain.server_id;
		}
		return server?.name ?? null;
	}

	function dnsBadge(check?: DnsCheckState) {
		if (check?.loading)
			return {
				label: t('projects.application_domains.checking_dns'),
				class: 'bg-muted text-muted-foreground',
				icon: LoaderCircleIcon,
			};
		if (check?.requestError || check?.result?.status === 'error')
			return {
				label: t('projects.application_domains.dns_check_failed'),
				class: 'bg-destructive/10 text-destructive',
				icon: TriangleAlertIcon,
			};
		const result = check?.result;
		if (result?.status === 'valid')
			return {
				label: t('projects.application_domains.dns_valid'),
				class: 'bg-success/10 text-success',
				icon: CheckIcon,
			};
		if (result?.status === 'cdn')
			return {
				label: t('projects.application_domains.dns_valid_cdn', {
					provider: result.cdn_provider ?? 'CDN',
				}),
				class: 'bg-success/10 text-success',
				icon: CheckIcon,
			};
		if (result)
			return {
				label: t('projects.application_domains.dns_mismatch'),
				class: 'bg-warning/10 text-warning',
				icon: TriangleAlertIcon,
			};
		return {
			label: t('projects.application_domains.check_dns'),
			class: 'bg-muted text-muted-foreground',
			icon: ShieldCheckIcon,
		};
	}
	async function validateDns(domain: DomainDto) {
		dnsChecks = { ...dnsChecks, [domain.id]: { loading: true } };
		try {
			const result = await onValidateDns(domain.id);
			dnsChecks = { ...dnsChecks, [domain.id]: { loading: false, result } };
		} catch (caught) {
			dnsChecks = {
				...dnsChecks,
				[domain.id]: {
					loading: false,
					requestError:
						caught instanceof Error
							? caught.message
							: t('projects.application_domains.could_not_validate_dns'),
				},
			};
		}
	}
	async function submit(
		input: DomainInput & {
			service?: string;
			container_port?: number;
		},
	) {
		const saved = editing ? await onUpdate(editing.id, input) : await onCreate(input);
		const domainId = editing?.id ?? saved?.id;
		if (input.service && input.container_port && domainId && onSaveComposeRoute)
			await onSaveComposeRoute(domainId, input.service, input.container_port);
	}
</script>

<div class="grid gap-4 max-w-full">
	<div class="flex flex-wrap items-center justify-between gap-3">
		<div>
			<p class="font-medium">{t('projects.application.domains')}</p>
			<p class="text-sm text-muted-foreground">{t('projects.application_domains.description')}</p>
		</div>
		<div class="flex gap-2">
			{#if server && server.kind === 'ssh'}
				<Button
					variant="outline"
					size="icon-sm"
					onclick={() => (dnsHelpOpen = true)}
					aria-label={t('projects.application_domains.dns_help')}
					title={t('projects.application_domains.dns_help')}
					><CircleHelpIcon class="size-4" /></Button>
			{/if}
			<Button variant="outline" disabled={busy || refreshing} onclick={() => void onRefresh()}
				><RefreshCwIcon class={refreshing ? 'size-4 animate-spin' : 'size-4'} />{t(
					'projects.application_general.refresh',
				)}</Button>
			<Button variant="outline" disabled={busy} onclick={() => void onGenerate()}
				>{t('projects.dialogs_common.random_sslip')}</Button>
			<Button disabled={busy} onclick={add}>{t('projects.dialogs_common.add_domain')}</Button>
		</div>
	</div>

	{#if domainMarkers?.requires_https}
		<div
			class="flex items-start gap-2 rounded-lg border border-warning/40 bg-warning/5 p-3 text-sm">
			<TriangleAlertIcon class="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
			<span>{t('projects.application_domains.requires_https')}</span>
		</div>
	{/if}
	{#if domainMarkers?.url_persisted}
		<div class="flex items-start gap-2 rounded-lg border p-3 text-sm text-muted-foreground">
			<CircleHelpIcon class="mt-0.5 size-4 shrink-0" aria-hidden="true" />
			<span>{t('projects.application_domains.url_persisted')}</span>
		</div>
	{/if}

	{#if configStale}
		<div
			class="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-warning/40 bg-warning/5 p-3 text-sm">
			<span>{t('projects.application_domains.config_stale')}</span>
			{#if allowedHere('app.deploy')}
				<Button size="sm" disabled={busy} onclick={() => void onDeploy()}
					>{t('projects.environment.redeploy')}</Button>
			{/if}
		</div>
	{/if}

	{#if domains.length === 0}
		<p class="rounded-lg border border-dashed p-6 text-center text-sm text-muted-foreground">
			{t('projects.application_domains.empty')}
		</p>
	{:else}
		<div class="lp-raised overflow-hidden rounded-xl border bg-card">
			<div
				class="lp-groove-b hidden grid-cols-[minmax(0,1.6fr)_minmax(0,1fr)_minmax(0,1fr)_2rem] gap-4 px-4 py-2 text-xs text-muted-foreground sm:grid">
				<span>{t('projects.application_domains.col_domain')}</span>
				<span>{t('projects.application_domains.col_target')}</span>
				<span>{t('projects.application_domains.col_certificate')}</span>
				<span></span>
			</div>
			{#each domains as domain, index (domain.id)}
				{@const composeRoute = composeRoutes.find((candidate) => candidate.domain_id === domain.id)}
				{@const dnsCheck = dnsChecks[domain.id]}
				<!-- The pencil's ::after covers the row, so the whole row edits;
				     links and the DNS button sit above it with `relative z-10`. -->
				<div
					class={`relative grid grid-cols-[minmax(0,1fr)_auto] gap-x-3 gap-y-2 px-4 py-3 transition-colors hover:bg-muted/40 sm:grid-cols-[minmax(0,1.6fr)_minmax(0,1fr)_minmax(0,1fr)_2rem] sm:items-center sm:gap-4 ${index ? 'lp-groove-t' : ''}`}>
					<div class="col-start-1 row-start-1 min-w-0 sm:col-start-auto sm:row-start-auto">
						<div class="flex min-w-0 flex-wrap items-center gap-2">
							<a
								class={`relative z-10 break-all font-medium underline-offset-4 hover:underline ${domain.host.endsWith('.sslip.io') ? 'font-mono text-[13px]' : ''}`}
								href={domain.url}
								target="_blank"
								rel="noreferrer">{domain.host}</a>
							{#if domain.is_primary}<span
									class="rounded-full bg-primary/10 px-2 py-0.5 text-xs font-medium text-primary"
									>{t('projects.dialogs_common.primary')}</span
								>{/if}
						</div>
						<div class="mt-1.5 flex flex-wrap gap-1.5 text-xs">
							{#if serverNameOf(domain)}<span
									class="flex items-center gap-1 rounded-full border px-2 py-0.5 font-medium"
									><ServerIcon class="size-3 text-muted-foreground" />{serverNameOf(domain)}</span
								>{/if}
							<span class="rounded-full bg-muted px-2 py-0.5 font-medium"
								>{domain.is_local
									? t('projects.application_domains.local')
									: domain.ingress_mode === 'public_edge'
										? t('projects.application_domains.direct_edge')
										: domain.ingress_mode === 'private_relay'
											? t('projects.application_domains.private_relay')
											: t('projects.application_domains.central_caddy')}</span>
							<span
								class={`rounded-full px-2 py-0.5 font-medium ${
									domain.route_health === 'ready'
										? 'bg-success/10 text-success'
										: 'bg-warning/10 text-warning'
								}`}>{domain.route_health}</span>
							{#if canValidateDns(domain)}
								{@const dns = dnsBadge(dnsCheck)}
								{@const DnsIcon = dns.icon}
								<button
									type="button"
									disabled={busy || dnsCheck?.loading}
									onclick={() => void validateDns(domain)}
									title={dnsCheck?.result?.message ??
										dnsCheck?.requestError ??
										t('projects.application_domains.check_dns_records')}
									class={`relative z-10 flex cursor-pointer items-center gap-1 rounded-full px-2 py-0.5 font-medium transition-colors hover:opacity-80 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none disabled:cursor-not-allowed disabled:opacity-60 ${dns.class}`}>
									<DnsIcon
										class={dnsCheck?.loading
											? 'size-3 animate-spin motion-reduce:animate-none'
											: 'size-3'} />
									{dns.label}
								</button>
							{/if}
						</div>
						{#if markerVariables(domain.id).length > 0}
							<p class="mt-2 text-xs text-muted-foreground">
								{t('projects.application_domains.used_by')}
								<span class="font-mono">{markerVariables(domain.id).join(', ')}</span>
							</p>
						{/if}
						{#if domain.required_dns_target}
							<p class="mt-2 text-xs text-muted-foreground">
								{t('projects.application_domains.dns_target')}
								<code class="font-mono text-foreground">{domain.required_dns_target}</code>
							</p>
						{/if}
						{#if domain.route_health_reason}
							<p class="mt-1 text-xs text-muted-foreground">{domain.route_health_reason}</p>
						{/if}
						{#if !dnsCheck?.loading && dnsCheck?.result}
							{@const result = dnsCheck.result}
							{#if result.status === 'valid' || result.status === 'cdn'}
								{#if result.message}<p class="mt-2 text-xs text-muted-foreground">
										{result.message}
									</p>{/if}
							{:else}
								<p class="mt-2 text-xs text-warning">
									{result.message ?? t('projects.application_domains.dns_not_pointing')}
								</p>
							{/if}
						{:else if !dnsCheck?.loading && dnsCheck?.requestError}
							<p class="mt-2 text-xs text-destructive">{dnsCheck.requestError}</p>
						{/if}
					</div>
					<div class="col-span-2 row-start-2 flex min-w-0 flex-wrap items-center gap-2 sm:contents">
						<span class="min-w-0">
							<span
								class="inline-block max-w-full truncate rounded-md bg-muted px-2 py-0.5 align-middle font-mono text-xs text-muted-foreground"
								title={composeRoute
									? t('projects.application_domains.caddy_route', {
											service: composeRoute.service,
											port: composeRoute.container_port,
										})
									: undefined}
								>{composeRoute
									? `${composeRoute.service}:${composeRoute.container_port}`
									: `:${domain.port}`}</span>
						</span>
						<span class="min-w-0">
							<span
								class={`inline-flex max-w-full items-center gap-1 rounded-full px-2 py-0.5 text-xs font-medium ${domain.https ? 'bg-success/10 text-success' : 'bg-muted text-muted-foreground'}`}
								>{#if domain.https}<LockIcon class="size-3 shrink-0" />{:else}<LockOpenIcon
										class="size-3 shrink-0" />{/if}<span class="truncate"
									>{certificateLabel(domain)}</span
								></span>
						</span>
					</div>
					<button
						type="button"
						class="col-start-2 row-start-1 grid size-8 place-items-center self-start rounded-full text-muted-foreground transition-colors after:absolute after:inset-0 after:content-[''] hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none disabled:opacity-50 sm:col-start-auto sm:row-start-auto sm:self-center"
						disabled={busy}
						aria-label={`${t('projects.common.edit')} ${domain.host}`}
						onclick={() => edit(domain)}><PencilIcon class="size-4" /></button>
				</div>
			{/each}
		</div>
	{/if}
</div>

{#if server && server.kind === 'ssh'}
	<DomainDnsHelpDialog bind:open={dnsHelpOpen} {server} {domains} />
{/if}

<DomainFormDialog
	bind:open={formOpen}
	domain={editing}
	{constraints}
	{pinServers}
	{onRandomSslipHost}
	onCheckDns={onValidateHostDns}
	{composeServices}
	serverAddress={server?.public_address ?? null}
	composeRoute={editing
		? (composeRoutes.find((route) => route.domain_id === editing?.id) ?? null)
		: null}
	deleteWarning={editing && orphanedByDelete(editing.id).length > 0
		? t('projects.application_domains.delete_marker_warning', {
				vars: orphanedByDelete(editing.id).join(', '),
			})
		: undefined}
	onDelete={editing ? () => onDelete(editing!.id) : undefined}
	onSubmit={submit} />
