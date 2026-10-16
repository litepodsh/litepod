<script lang="ts">
	import CertificateFields from '#lib/components/custom/certificate-fields.svelte';
	import type { CertificateProvider } from '#lib/clients/server-settings-client.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import * as Select from '#lib/components/ui/select/index.js';
	import SoftSwitch from '#lib/components/custom/soft-switch.svelte';
	import CopyButton from '#lib/components/custom/copy-button.svelte';
	import DeleteConfirmControl from '#lib/components/custom/delete-confirm-control.svelte';
	import SearchableDropdown, {
		type SearchableDropdownItem,
	} from '#lib/components/custom/searchable-dropdown.svelte';
	import type {
		DomainDnsValidationResult,
		DomainDto,
		DomainInput,
		PortConstraintsDto,
		ServerSummaryDto,
	} from '#lib/clients/project-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import BoxIcon from '@lucide/svelte/icons/box';
	import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
	import CircleCheckIcon from '@lucide/svelte/icons/circle-check';
	import DatabaseIcon from '@lucide/svelte/icons/database';
	import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import ShuffleIcon from '@lucide/svelte/icons/shuffle';
	import XIcon from '@lucide/svelte/icons/x';

	let {
		open = $bindable(false),
		domain = null,
		constraints = { reserved_ports: [], taken_ports: [] },
		pinServers = [],
		onRandomSslipHost,
		onCheckDns,
		composeServices = [],
		composeRoute = null,
		serverAddress = null,
		deleteWarning,
		onDelete,
		onSubmit,
	}: {
		open?: boolean;
		domain?: DomainDto | null;
		constraints?: PortConstraintsDto;
		/** Members of a group-targeted application; non-empty when the domain must be pinned. */
		pinServers?: ServerSummaryDto[];
		onRandomSslipHost?: () => Promise<string>;
		/** Resolves the typed host against the server(s) it should point to. */
		onCheckDns?: (host: string, serverId: string | null) => Promise<DomainDnsValidationResult>;
		composeServices?: { name: string; suggested_internal_ports: number[]; image?: string }[];
		composeRoute?: { service: string; container_port: number } | null;
		/** Public address of the serving server, shown as the DNS record value for a new host. */
		serverAddress?: string | null;
		/** Shown above Remove, e.g. variables left without a domain. */
		deleteWarning?: string;
		/** Present in edit mode to offer Remove. */
		onDelete?: () => Promise<void>;
		onSubmit: (
			input: DomainInput & {
				service?: string;
				container_port?: number;
			},
		) => Promise<void>;
	} = $props();

	let host = $state('');
	// `Input type="number"` binds a number, not a string, and an empty field
	// binds undefined — so never assume this is a string.
	let port = $state<string | number | undefined>('');
	let https = $state(false);
	let primary = $state(false);
	let certificateProvider = $state<CertificateProvider>('automatic');
	let certificateId = $state('');
	let service = $state<string | null>(null);
	let containerPort = $state<string | number | undefined>('');
	let serverId = $state('');
	let serviceItems = $derived<SearchableDropdownItem[]>(
		composeServices.map((item) => ({
			value: item.name,
			label: item.name,
			icon: /postgres|mysql|maria|mongo|redis|valkey|db/i.test(`${item.name} ${item.image ?? ''}`)
				? 'database'
				: 'box',
			description: [
				item.image,
				item.suggested_internal_ports.length
					? t('projects.domain_form.ports_label', {
							ports: item.suggested_internal_ports.join(', '),
						})
					: t('projects.domain_form.no_declared_port'),
			]
				.filter(Boolean)
				.join(' · '),
		})),
	);

	function parsePort(value: string | number | undefined): number | null {
		if (value === undefined || value === null) return null;
		const trimmed = String(value).trim();
		if (!trimmed) return null;
		const parsed = Number(trimmed);
		return Number.isFinite(parsed) ? parsed : null;
	}
	let saving = $state(false);
	let wasOpen = $state(false);
	let generatingSslip = $state(false);

	const normalizedHost = $derived(host.trim().toLowerCase());
	const isLocal = $derived(['localhost', '127.0.0.1', '::1'].includes(normalizedHost));
	// Public domains always enter through Caddy. This is deliberately separate
	// from `port`, which is the port Caddy dials inside the container network.
	const publicPort = $derived(https ? 443 : 80);

	// Constraints are a snapshot, so the server stays the authority — this only
	// spares the user a round trip for the failures we can already see.
	const portError = $derived.by(() => {
		const parsed = parsePort(port);
		// An untouched field is not yet a mistake — emptiness is caught on submit.
		if (parsed === null) return '';
		if (!Number.isInteger(parsed) || parsed < 1 || parsed > 65535)
			return t('projects.domain_form.port_error_range');
		if (!isLocal) return '';
		if (constraints.reserved_ports.includes(parsed))
			return t('projects.domain_form.port_error_reserved', { port: parsed });
		if (parsed !== domain?.port && constraints.taken_ports.includes(parsed))
			return t('projects.domain_form.port_error_taken', { port: parsed });
		return '';
	});

	const reservedList = $derived(constraints.reserved_ports.join(', '));

	// DNS card: the record to create comes from the domain (or the server's
	// address for a new host); `onCheckDns` asks the API whether it resolves.
	type DnsCheck = { loading: boolean; result?: DomainDnsValidationResult; error?: string };
	let dnsCheck = $state<DnsCheck>({ loading: false });
	let dnsRun = 0;
	const isSslip = $derived(normalizedHost.endsWith('.sslip.io'));
	const canCheckDns = $derived(
		open &&
			Boolean(onCheckDns) &&
			!isLocal &&
			!isSslip &&
			/^([a-z0-9-]+\.)+[a-z][a-z0-9-]+$/.test(normalizedHost),
	);
	const dnsTarget = $derived(
		domain?.required_dns_target ?? serverAddress ?? dnsCheck.result?.expected_addresses[0] ?? '',
	);
	const dnsFound = $derived(
		[
			...(dnsCheck.result?.cname ? [`CNAME ${dnsCheck.result.cname}`] : []),
			...(dnsCheck.result?.resolved_addresses ?? []).map(
				(address) => `${address.includes(':') ? 'AAAA' : 'A'} ${address}`,
			),
		].join(', '),
	);

	async function checkDns(checked: string, pin: string) {
		if (!onCheckDns) return;
		const run = ++dnsRun;
		dnsCheck = { loading: true };
		try {
			const result = await onCheckDns(checked, pin || null);
			if (run === dnsRun) dnsCheck = { loading: false, result };
		} catch (error) {
			if (run === dnsRun)
				dnsCheck = {
					loading: false,
					error: error instanceof Error ? error.message : t('projects.domain_form.dns_error'),
				};
		}
	}

	// Debounced so typing a host does not fire one lookup per keystroke.
	$effect(() => {
		const checked = normalizedHost;
		const pin = serverId;
		if (!canCheckDns) {
			dnsRun++;
			dnsCheck = { loading: false };
			return;
		}
		const timer = setTimeout(() => void checkDns(checked, pin), 600);
		return () => clearTimeout(timer);
	});
	const dnsRecordType = $derived(
		/^\d{1,3}(\.\d{1,3}){3}$/.test(dnsTarget) ? 'A' : dnsTarget.includes(':') ? 'AAAA' : 'CNAME',
	);
	const dnsRecordName = $derived(
		normalizedHost.split('.').length > 2 ? normalizedHost.split('.').slice(0, -2).join('.') : '@',
	);
	const meta = $derived(
		[
			isLocal
				? t('projects.application_domains.local')
				: `${https ? 'HTTPS' : 'HTTP'} · ${publicPort}`,
			service && parsePort(containerPort) !== null
				? `${service}:${parsePort(containerPort)}`
				: !composeServices.length && parsePort(port) !== null
					? `:${parsePort(port)}`
					: '',
			pinServers.find((server) => server.id === serverId)?.name ?? '',
		].filter(Boolean),
	);

	$effect(() => {
		if (open && !wasOpen) {
			host = domain?.host ?? '';
			port = domain?.port?.toString() ?? '';
			https = domain?.https ?? false;
			primary = domain?.is_primary ?? false;
			certificateProvider = domain?.certificate_provider ?? 'automatic';
			certificateId = domain?.certificate_id ?? '';
			service = composeRoute?.service ?? null;
			containerPort = composeRoute?.container_port?.toString() ?? '';
			serverId = domain?.server_id ?? (pinServers.length === 1 ? pinServers[0].id : '');
		}
		wasOpen = open;
	});

	// Caddy cannot issue a certificate for a local host, so the toggle cannot stay on.
	$effect(() => {
		if (isLocal && https) https = false;
	});

	async function submit() {
		const trimmedHost = normalizedHost;
		if (!trimmedHost) {
			toast.error(t('projects.domain_form.host_required'));
			return;
		}

		const selectedPort = parsePort(containerPort);
		if (composeServices.length && (!service || selectedPort === null)) {
			toast.error(t('projects.domain_form.caddy_target_required'));
			return;
		}

		// A Compose route's container port IS the port Caddy dials - the domain
		// row's own `port` column just mirrors it so it stays populated for the
		// (host, port) uniqueness check; nothing routing-related reads it.
		const effectivePort = composeServices.length ? selectedPort : parsePort(port);
		if (effectivePort === null) {
			toast.error(t('projects.domain_form.port_required'));
			return;
		}
		if (!composeServices.length && portError) {
			toast.error(portError);
			return;
		}

		if (pinServers.length && !serverId) {
			toast.error(t('projects.domain_form.pin_required'));
			return;
		}

		saving = true;
		try {
			await onSubmit({
				host: trimmedHost,
				port: effectivePort,
				https,
				certificate_provider: https ? certificateProvider : 'automatic',
				...(https && certificateProvider === 'custom' ? { certificate_id: certificateId } : {}),
				...(pinServers.length ? { server_id: serverId } : {}),
				// Only promote: the API keeps one primary per app, and un-marking it
				// here would leave the app without one.
				...(primary && !domain?.is_primary ? { is_primary: true } : {}),
				service: service ?? undefined,
				container_port: selectedPort ?? undefined,
			});
			open = false;
		} catch (error) {
			toast.error(error instanceof Error ? error.message : t('projects.domain_form.failed_save'));
		} finally {
			saving = false;
		}
	}

	async function remove() {
		if (!onDelete) return;
		try {
			await onDelete();
			open = false;
		} catch (error) {
			toast.error(error instanceof Error ? error.message : t('projects.domain_form.failed_remove'));
		}
	}

	async function useRandomSslip() {
		if (!onRandomSslipHost) return;
		generatingSslip = true;
		try {
			host = await onRandomSslipHost();
		} catch (error) {
			toast.error(error instanceof Error ? error.message : t('projects.domain_form.sslip_failed'));
		} finally {
			generatingSslip = false;
		}
	}
</script>

{#snippet serviceIcon(name: string)}
	<span
		class="lp-raised grid size-6 shrink-0 place-items-center rounded-md bg-muted text-muted-foreground">
		{#if name === 'database'}<DatabaseIcon class="size-3.5" />{:else}<BoxIcon
				class="size-3.5" />{/if}
	</span>
{/snippet}

<Dialog.Root bind:open>
	<Dialog.Content
		showCloseButton={false}
		class="flex max-h-[min(88dvh,880px)] flex-col gap-0 overflow-hidden p-0 sm:max-w-xl">
		<header
			class="lp-groove-b grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-3 gap-y-0.5 py-3 ps-5 pe-3.5">
			<p class="font-mono text-[10.5px] tracking-[.06em] text-muted-foreground uppercase">
				{domain ? t('projects.domain_form.edit_domain') : t('projects.dialogs_common.add_domain')}
			</p>
			<Dialog.Close>
				{#snippet child({ props })}
					<Button
						{...props}
						variant="ghost"
						size="icon-sm"
						class="row-span-2 self-start bg-secondary"
						aria-label={t('common.common.close')}><XIcon /></Button>
				{/snippet}
			</Dialog.Close>
			<Dialog.Title class="truncate text-base leading-tight font-semibold tracking-tight">
				{normalizedHost || t('projects.domain_form.host_placeholder')}
			</Dialog.Title>
			<Dialog.Description
				class="col-span-2 mt-0.5 flex flex-wrap gap-x-2.5 gap-y-1 font-mono text-xs text-muted-foreground">
				{#each meta as item (item)}<span>{item}</span>{/each}
			</Dialog.Description>
		</header>

		<form
			class="flex min-h-0 flex-1 flex-col"
			onsubmit={(event) => {
				event.preventDefault();
				void submit();
			}}>
			<div class="grid min-h-0 flex-1 gap-5 overflow-y-auto px-5 py-4">
				<section class="grid gap-3">
					<div class="grid gap-2">
						<Label for="domain-host">{t('projects.domain_form.host')}</Label>
						<div class="flex gap-2">
							<Input
								id="domain-host"
								class="flex-1 font-mono"
								bind:value={host}
								maxlength={253}
								autocomplete="off"
								placeholder={t('projects.domain_form.host_placeholder')}
								autofocus />
							{#if onRandomSslipHost}<Button
									type="button"
									variant="outline"
									class="shrink-0"
									disabled={generatingSslip || saving}
									onclick={useRandomSslip}
									><ShuffleIcon /><span class="max-sm:sr-only"
										>{generatingSslip
											? t('projects.domain_form.generating')
											: t('projects.dialogs_common.random_sslip')}</span
									></Button
								>{/if}
						</div>
						<p class="text-xs text-muted-foreground">{t('projects.domain_form.description')}</p>
					</div>
					{#if normalizedHost && !isLocal && (dnsTarget || isSslip || canCheckDns)}
						{@const result = dnsCheck.result}
						<div class="lp-well grid gap-2 rounded-[10px] bg-muted p-3 text-xs">
							<div class="flex flex-wrap items-center justify-between gap-2">
								<span class="font-semibold">DNS</span>
								<span class="flex flex-wrap items-center gap-2" aria-live="polite">
									{#if isSslip}
										<span class="flex items-center gap-1.5 text-success"
											><CircleCheckIcon class="size-3.5" />{t(
												'projects.domain_form.dns_sslip',
											)}</span>
									{:else if dnsCheck.loading}
										<span class="flex items-center gap-1.5 text-muted-foreground"
											><LoaderCircleIcon
												class="size-3.5 animate-spin motion-reduce:animate-none" />{t(
												'projects.domain_form.dns_checking',
											)}</span>
									{:else if dnsCheck.error || result?.status === 'error'}
										<span
											class="flex items-center gap-1.5 text-destructive"
											title={dnsCheck.error ?? result?.message ?? undefined}
											><CircleAlertIcon class="size-3.5" />{t(
												'projects.domain_form.dns_error',
											)}</span>
									{:else if result?.status === 'valid'}
										<span class="flex items-center gap-1.5 text-success"
											><CircleCheckIcon class="size-3.5" />{t('projects.domain_form.dns_ok', {
												target: result.resolved_addresses.join(', '),
											})}</span>
									{:else if result?.status === 'cdn'}
										<span class="flex items-center gap-1.5 text-success"
											><CircleCheckIcon class="size-3.5" />{t('projects.domain_form.dns_cdn', {
												provider: result.cdn_provider ?? 'CDN',
											})}</span>
									{:else if result?.status === 'invalid'}
										<span class="flex items-center gap-1.5 text-warning"
											><CircleAlertIcon class="size-3.5" />{t(
												'projects.domain_form.dns_mismatch',
											)}</span>
									{:else if result?.status === 'unresolved'}
										<span class="flex items-center gap-1.5 text-warning"
											><CircleAlertIcon class="size-3.5" />{t(
												'projects.domain_form.dns_missing',
											)}</span>
									{/if}
									{#if canCheckDns && !dnsCheck.loading && (result || dnsCheck.error)}
										<Button
											type="button"
											variant="outline"
											size="sm"
											class="h-7 px-2 text-xs"
											onclick={() => void checkDns(normalizedHost, serverId)}
											><RefreshCwIcon class="size-3.5" />{t(
												'projects.domain_form.dns_check_again',
											)}</Button>
									{/if}
								</span>
							</div>
							{#if !isSslip && !dnsCheck.loading && (dnsCheck.error || result?.status === 'error')}
								<p class="break-words text-muted-foreground">
									{dnsCheck.error ?? result?.message}
								</p>
							{:else if !isSslip && !dnsCheck.loading && dnsFound && result?.status !== 'valid'}
								<p class="font-mono break-all text-muted-foreground">
									{t('projects.domain_form.dns_found', { records: dnsFound })}
								</p>
							{/if}
							{#if !isSslip && dnsTarget}
								<div
									class="grid grid-cols-[3.5rem_minmax(0,1fr)_minmax(0,1.4fr)] items-center gap-x-2.5 gap-y-1 font-mono">
									<span class="font-sans text-[11.5px] text-muted-foreground"
										>{t('projects.domain_form.dns_type')}</span>
									<span class="font-sans text-[11.5px] text-muted-foreground"
										>{t('projects.domain_form.dns_name')}</span>
									<span class="font-sans text-[11.5px] text-muted-foreground"
										>{t('projects.domain_dns.value')}</span>
									<span>{dnsRecordType}</span>
									<span class="truncate">{dnsRecordName}</span>
									<CopyButton
										value={dnsTarget}
										revealOnHover={false}
										class="min-w-0 justify-between"
										errorMessage={t('projects.domain_dns.copy_failed')} />
								</div>
							{/if}
						</div>
					{/if}
				</section>

				{#if composeServices.length}
					<section class="lp-groove-t grid gap-3 pt-4">
						<div class="grid gap-2">
							<Label>{t('projects.domain_form.caddy_target_service')}</Label><SearchableDropdown
								items={serviceItems}
								bind:selectedValue={service}
								label={t('projects.domain_form.search_compose_services')}
								searchPlaceholder={t('projects.domain_form.search_services')}
								emptyText={t('projects.application_compose.no_service_found')}
								icon={serviceIcon}
								triggerClass="h-10 w-full font-mono [&>span.truncate]:flex-1 [&>span.truncate]:text-left"
								onSelect={(selected) => {
									const found = composeServices.find((item) => item.name === selected);
									if (found?.suggested_internal_ports[0])
										containerPort = found.suggested_internal_ports[0];
								}} />
							<p class="text-xs text-muted-foreground">
								{t('projects.domain_form.caddy_route_hint')}
							</p>
						</div>
						<div class="grid gap-2">
							<Label for="domain-container-port">{t('projects.domain_form.internal_port')}</Label
							><Input
								id="domain-container-port"
								class="max-w-40 font-mono"
								type="number"
								min="1"
								max="65535"
								bind:value={containerPort}
								placeholder="3000" />
							<p class="text-xs text-muted-foreground">
								{t('projects.domain_form.internal_port_hint')}
							</p>
						</div>
					</section>
				{:else}
					<!-- A Compose route already carries its own container port (above);
					     asking for this too just duplicates it, unused, on the domain row. -->
					<section class="lp-groove-t grid gap-2 pt-4">
						<Label for="domain-port"
							>{isLocal
								? t('projects.domain_form.port')
								: t('projects.domain_form.internal_port_caddy')}</Label>
						<Input
							id="domain-port"
							class="max-w-40 font-mono"
							type="number"
							min="1"
							max="65535"
							bind:value={port}
							placeholder={isLocal ? '3000' : '80'}
							aria-invalid={portError ? 'true' : undefined} />
						{#if portError}
							<p class="text-xs text-destructive">{portError}</p>
						{:else}
							<p class="text-xs text-muted-foreground">
								{isLocal
									? t('projects.domain_form.local_hint', {
											reserved: reservedList
												? t('projects.domain_form.local_reserved', { list: reservedList })
												: '',
										})
									: t('projects.domain_form.public_hint', {
											protocol: https ? 'HTTPS' : 'HTTP',
											port: publicPort,
										})}
							</p>
						{/if}
					</section>
				{/if}

				<section class="lp-groove-t grid gap-4 pt-4">
					<div class="flex items-center justify-between gap-3">
						<div>
							<Label for="domain-https">HTTPS</Label>
							<p class="mt-1 text-xs text-muted-foreground">
								{isLocal
									? t('projects.domain_form.https_unavailable_local')
									: t('projects.domain_form.https_public_hint', { port: publicPort })}
							</p>
						</div>
						<SoftSwitch id="domain-https" bind:checked={https} disabled={isLocal} />
					</div>
					{#if https}
						<CertificateFields
							bind:provider={certificateProvider}
							bind:certificateId
							certificateName={domain?.certificate_name ?? ''}
							disabled={saving} />
					{/if}
					<div class="flex items-center justify-between gap-3">
						<div>
							<Label for="domain-primary">{t('projects.domain_form.primary')}</Label>
							<p class="mt-1 text-xs text-muted-foreground">
								{t('projects.domain_form.primary_hint')}
							</p>
						</div>
						<SoftSwitch id="domain-primary" bind:checked={primary} disabled={domain?.is_primary} />
					</div>
					{#if pinServers.length}
						<div class="grid gap-2">
							<Label for="domain-pin">{t('projects.domain_form.pin')}</Label>
							<Select.Root type="single" bind:value={serverId}>
								<Select.Trigger id="domain-pin" class="w-full">
									{pinServers.find((server) => server.id === serverId)?.name ??
										t('projects.application_form.choose_server')}
								</Select.Trigger>
								<Select.Content>
									{#each pinServers as server (server.id)}
										<Select.Item value={server.id}>{server.name}</Select.Item>
									{/each}
								</Select.Content>
							</Select.Root>
							<p class="text-xs text-muted-foreground">{t('projects.domain_form.pin_hint')}</p>
						</div>
					{/if}
				</section>
				{#if domain && onDelete && deleteWarning}
					<p class="rounded-lg border border-warning/40 bg-warning/5 p-3 text-xs">
						{deleteWarning}
					</p>
				{/if}
			</div>

			<footer
				class="lp-groove-t flex flex-wrap items-center justify-between gap-2 bg-card px-5 py-3">
				{#if domain && onDelete}
					<DeleteConfirmControl
						label={t('projects.domain_form.remove')}
						pendingLabel={t('projects.domain_form.removing')}
						disabled={saving}
						class="w-full sm:h-8 sm:w-auto"
						onconfirm={remove} />
				{:else}
					<span></span>
				{/if}
				<div class="flex gap-2 max-sm:w-full max-sm:*:flex-1">
					<Button type="button" variant="outline" onclick={() => (open = false)}
						>{t('projects.projects.cancel')}</Button>
					<Button type="submit" disabled={saving}
						>{saving
							? t('projects.projects.saving')
							: domain
								? t('projects.projects.save_changes')
								: t('projects.dialogs_common.add_domain')}</Button>
				</div>
			</footer>
		</form>
	</Dialog.Content>
</Dialog.Root>
