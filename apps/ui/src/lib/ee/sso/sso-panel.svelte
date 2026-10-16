<script lang="ts">
	import { onMount } from 'svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import { NativeSelect } from '#lib/components/ui/native-select/index.js';
	import { Switch } from '#lib/components/ui/switch/index.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import SoftCheckbox from '#lib/components/custom/soft-checkbox.svelte';
	import SkeletonRows from '#lib/components/custom/skeleton-rows.svelte';
	import SsoFieldInfo from './sso-field-info.svelte';
	import type { SsoHelpField } from './sso-field-help';
	import CopyIcon from '@lucide/svelte/icons/copy';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import {
		addDomain,
		assignDomain,
		deleteConnection,
		deleteDomain,
		disableScim,
		getSsoSettings,
		listConnections,
		listDomains,
		listMappings,
		rotateScimToken,
		saveConnection,
		setEnforcement,
		setMappings,
		verifyDomain,
		type GroupMapping,
		type ScimToken,
		type SsoConnection,
		type SsoConnectionInput,
		type SsoDomain,
		type SsoSettings,
	} from './sso-client';
	import { listGroups, type Group } from '#lib/clients/access-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let { organizationId }: { organizationId: string } = $props();

	let domains = $state<SsoDomain[]>([]);
	let connections = $state<SsoConnection[]>([]);
	let groups = $state<Group[]>([]);
	let settings = $state<SsoSettings>({
		enforced: false,
		scim_enabled: false,
		saml_supported: false,
	});
	let loading = $state(true);
	let busy = $state(false);
	let newDomain = $state('');
	let scimToken = $state<ScimToken | null>(null);

	// Connection editor
	let editorOpen = $state(false);
	let editing = $state<SsoConnection | null>(null);
	let form = $state<SsoConnectionInput>(blankForm());

	// Group mappings editor
	let mappingFor = $state<SsoConnection | null>(null);
	let mappings = $state<GroupMapping[]>([]);

	/** Entra's SAML group attribute; OIDC uses the `groups` claim. */
	const ENTRA_SAML_GROUPS = 'http://schemas.microsoft.com/ws/2008/06/identity/claims/groups';
	const BASE_ROLES: [number, string][] = [
		[1, 'Admin'],
		[2, 'Editor'],
		[3, 'Viewer'],
		[4, 'Member'],
	];
	const mappable = $derived(groups.filter((g) => g.source !== 'scim'));
	const anyEnabled = $derived(connections.some((c) => c.enabled));

	function blankForm(): SsoConnectionInput {
		return {
			kind: 'oidc',
			name: '',
			issuer: '',
			client_id: '',
			client_secret: '',
			idp_metadata_xml: '',
			groups_claim: 'groups',
			jit_enabled: true,
			default_base_role_id: 4,
			session_ttl_minutes: 60,
			enabled: false,
		};
	}

	function chooseKind(kind: 'oidc' | 'saml') {
		form.kind = kind;
		form.groups_claim = kind === 'saml' ? ENTRA_SAML_GROUPS : 'groups';
	}

	const formComplete = $derived(
		!!form.name.trim() &&
			(form.kind === 'oidc'
				? !!form.issuer.trim() && !!form.client_id.trim()
				: !!editing || !!form.idp_metadata_xml?.trim()) &&
			form.session_ttl_minutes >= 5 &&
			form.session_ttl_minutes <= 43200,
	);

	onMount(async () => {
		try {
			[domains, connections, groups, settings] = await Promise.all([
				listDomains(organizationId),
				listConnections(organizationId),
				listGroups(organizationId),
				getSsoSettings(organizationId),
			]);
		} catch (err) {
			fail(err);
		} finally {
			loading = false;
		}
	});

	function fail(err: unknown) {
		toast.error(err instanceof Error ? err.message : t('organizations.sso.failed'));
	}

	async function run(action: () => Promise<void>) {
		busy = true;
		try {
			await action();
		} catch (err) {
			fail(err);
		} finally {
			busy = false;
		}
	}

	function copy(value: string) {
		void navigator.clipboard.writeText(value);
		toast.success(t('organizations.sso.copied'));
	}

	const submitDomain = (event: SubmitEvent) => {
		event.preventDefault();
		return run(async () => {
			domains = [...domains, await addDomain(organizationId, newDomain)];
			newDomain = '';
		});
	};

	const verify = (domain: SsoDomain) =>
		run(async () => {
			const verified = await verifyDomain(organizationId, domain.id);
			domains = domains.map((d) => (d.id === verified.id ? verified : d));
			toast.success(t('organizations.sso.domain_verified', { domain: domain.domain }));
		});

	const removeDomain = (domain: SsoDomain) =>
		run(async () => {
			await deleteDomain(organizationId, domain.id);
			domains = domains.filter((d) => d.id !== domain.id);
		});

	// Empty secret / metadata = keep the stored ones.
	const inputOf = (connection: SsoConnection): SsoConnectionInput => ({
		kind: connection.kind,
		name: connection.name,
		issuer: connection.issuer ?? '',
		client_id: connection.client_id ?? '',
		client_secret: '',
		idp_metadata_xml: '',
		groups_claim: connection.groups_claim,
		jit_enabled: connection.jit_enabled,
		default_base_role_id: connection.default_base_role_id,
		session_ttl_minutes: connection.session_ttl_minutes,
		enabled: connection.enabled,
	});

	function openEditor(connection: SsoConnection | null) {
		editing = connection;
		form = connection ? inputOf(connection) : blankForm();
		editorOpen = true;
	}

	const toggleConnection = (connection: SsoConnection, enabled: boolean) =>
		run(async () => {
			const input = {
				...inputOf(connection),
				enabled,
				client_secret: undefined,
				idp_metadata_xml: undefined,
			};
			const saved = await saveConnection(organizationId, input, connection.id);
			connections = connections.map((c) => (c.id === saved.id ? saved : c));
		});

	const assign = (domain: SsoDomain, connectionId: string) =>
		run(async () => {
			const saved = await assignDomain(organizationId, domain.id, connectionId || null);
			domains = domains.map((d) => (d.id === saved.id ? saved : d));
		});

	const saveEditor = () =>
		run(async () => {
			const input = {
				...form,
				client_secret: form.client_secret?.trim() || undefined,
				idp_metadata_xml: form.idp_metadata_xml?.trim() || undefined,
			};
			const saved = await saveConnection(organizationId, input, editing?.id);
			connections = editing
				? connections.map((c) => (c.id === saved.id ? saved : c))
				: [...connections, saved];
			editorOpen = false;
			toast.success(t('organizations.sso.connection_saved'));
		});

	const removeConnection = (connection: SsoConnection) =>
		run(async () => {
			await deleteConnection(organizationId, connection.id);
			connections = connections.filter((c) => c.id !== connection.id);
		});

	const openMappings = (connection: SsoConnection) =>
		run(async () => {
			mappings = await listMappings(organizationId, connection.id);
			mappingFor = connection;
		});

	const saveMappings = () =>
		run(async () => {
			if (!mappingFor) return;
			const rows = mappings.filter((m) => m.external_group.trim() && m.group_id);
			mappings = await setMappings(organizationId, mappingFor.id, rows);
			groups = await listGroups(organizationId);
			mappingFor = null;
			toast.success(t('organizations.sso.mappings_saved'));
		});

	const rotateScim = () =>
		run(async () => {
			scimToken = await rotateScimToken(organizationId);
			settings = { ...settings, scim_enabled: true };
		});

	const turnOffScim = () =>
		run(async () => {
			await disableScim(organizationId);
			scimToken = null;
			settings = { ...settings, scim_enabled: false };
		});

	const toggleEnforcement = (enforced: boolean) =>
		run(async () => {
			settings = await setEnforcement(organizationId, enforced);
		});
</script>

{#snippet fieldLabel(id: string, label: string, field: SsoHelpField)}
	<div class="flex items-center gap-1.5">
		<Label for={id}>{label}</Label>
		<SsoFieldInfo {field} title={label} />
	</div>
{/snippet}

{#snippet infoLabel(label: string, field: SsoHelpField, extra = '')}
	<span class="flex items-center gap-1.5 text-muted-foreground {extra}">
		{label}
		<SsoFieldInfo {field} title={label} />
	</span>
{/snippet}

{#snippet copyable(value: string)}
	<span class="flex min-w-0 items-center gap-1">
		<code class="min-w-0 truncate rounded bg-muted px-1.5 py-0.5 text-xs">{value}</code>
		<Button
			variant="ghost"
			size="icon-sm"
			aria-label={t('organizations.sso.copy')}
			onclick={() => copy(value)}><CopyIcon class="size-3.5" /></Button>
	</span>
{/snippet}

{#if loading}
	<SkeletonRows rows={4} height="h-16" label={t('organizations.sso.loading')} />
{:else}
	<div class="space-y-10">
		<p class="text-sm text-muted-foreground">{t('organizations.sso.intro')}</p>

		<!-- Domains -->
		<section class="space-y-3" aria-labelledby="sso-domains">
			<div>
				<h3 id="sso-domains" class="text-[15px] font-semibold">{t('organizations.sso.domains')}</h3>
				<p class="mt-1 text-sm text-muted-foreground">{t('organizations.sso.domains_hint')}</p>
			</div>
			<form class="flex flex-wrap gap-2" onsubmit={submitDomain}>
				<Input bind:value={newDomain} placeholder="company.com" class="w-full sm:w-64" />
				<Button type="submit" disabled={busy || !newDomain.trim()}
					>{t('organizations.sso.add_domain')}</Button>
			</form>
			{#if domains.length}
				<ul class="lp-raised overflow-hidden rounded-2xl border bg-card">
					{#each domains as domain (domain.id)}
						<li class="grid gap-2 p-4 not-last:border-b not-last:border-border/60">
							<div class="flex flex-wrap items-center gap-2">
								<span class="font-medium">{domain.domain}</span>
								<Badge variant={domain.verified ? 'secondary' : 'outline'}>
									{domain.verified
										? t('organizations.sso.verified')
										: t('organizations.sso.pending')}
								</Badge>
								<div class="ms-auto flex items-center gap-1">
									{#if connections.length > 1}
										<NativeSelect
											aria-label={t('organizations.sso.domain_connection')}
											value={domain.connection_id ?? ''}
											disabled={busy}
											onchange={(e) => assign(domain, e.currentTarget.value)}
											class="h-8 w-48 text-sm">
											<option value="">{t('organizations.sso.domain_connection_auto')}</option>
											{#each connections as connection (connection.id)}
												<option value={connection.id}>{connection.name}</option>
											{/each}
										</NativeSelect>
									{/if}
									{#if !domain.verified}
										<Button
											size="sm"
											variant="outline"
											disabled={busy}
											onclick={() => verify(domain)}>{t('organizations.sso.verify')}</Button>
									{/if}
									<Button
										size="sm"
										variant="ghost"
										class="text-destructive hover:text-destructive"
										aria-label={t('organizations.sso.remove')}
										disabled={busy}
										onclick={() => removeDomain(domain)}><Trash2Icon class="size-4" /></Button>
								</div>
							</div>
							{#if !domain.verified}
								<div class="grid gap-1 text-sm text-muted-foreground">
									<span>{t('organizations.sso.txt_hint')}</span>
									<div class="grid gap-1 sm:grid-cols-[6rem_minmax(0,1fr)]">
										<span>{t('organizations.sso.txt_host')}</span>
										{@render copyable(domain.txt_host)}
										<span>{t('organizations.sso.txt_value')}</span>
										{@render copyable(domain.txt_value)}
									</div>
									<span class="text-xs"
										>{t('organizations.sso.txt_full_name_hint', { name: domain.txt_name })}</span>
								</div>
							{/if}
						</li>
					{/each}
				</ul>
			{/if}
		</section>

		<!-- Connections -->
		<section class="space-y-3" aria-labelledby="sso-connections">
			<div class="flex flex-wrap items-start justify-between gap-3">
				<div>
					<h3 id="sso-connections" class="text-[15px] font-semibold">{t('organizations.sso.connections')}</h3>
					<p class="mt-1 text-sm text-muted-foreground">
						{t('organizations.sso.connections_hint')}
					</p>
				</div>
				<Button size="sm" onclick={() => openEditor(null)}>
					<PlusIcon class="size-4" />{t('organizations.sso.new_connection')}
				</Button>
			</div>
			{#if connections.length === 0}
				<p class="text-sm text-muted-foreground">{t('organizations.sso.no_connections')}</p>
			{:else}
				<ul class="lp-raised overflow-hidden rounded-2xl border bg-card">
					{#each connections as connection (connection.id)}
						<li
							class="flex flex-col gap-3 p-4 not-last:border-b not-last:border-border/60 sm:flex-row sm:items-center">
							<div class="flex min-w-0 flex-1 flex-col">
								<span class="flex items-center gap-2 font-medium">
									{connection.name}
									<Badge variant="outline">{connection.kind.toUpperCase()}</Badge>
								</span>
								<span class="truncate text-sm text-muted-foreground"
									>{connection.issuer ?? connection.idp_entity_id}</span>
							</div>
							<div class="flex flex-wrap items-center gap-1">
								<label class="me-2 flex items-center gap-2 text-sm text-muted-foreground">
									<Switch
										checked={connection.enabled}
										disabled={busy}
										onCheckedChange={(on) => toggleConnection(connection, on)} />
									{connection.enabled
										? t('organizations.sso.enabled')
										: t('organizations.sso.disabled')}
								</label>
								<Button variant="ghost" size="sm" onclick={() => openEditor(connection)}
									>{t('organizations.sso.edit')}</Button>
								<Button
									variant="ghost"
									size="sm"
									disabled={busy}
									onclick={() => openMappings(connection)}
									>{t('organizations.sso.group_mappings')}</Button>
								<Button
									variant="ghost"
									size="sm"
									class="text-destructive hover:text-destructive"
									aria-label={t('organizations.sso.remove')}
									disabled={busy}
									onclick={() => removeConnection(connection)}
									><Trash2Icon class="size-4" /></Button>
							</div>
						</li>
					{/each}
				</ul>
			{/if}
		</section>

		<!-- SCIM -->
		<section class="space-y-3" aria-labelledby="sso-scim">
			<div>
				<h3 id="sso-scim" class="text-[15px] font-semibold">{t('organizations.sso.scim')}</h3>
				<p class="mt-1 text-sm text-muted-foreground">{t('organizations.sso.scim_hint')}</p>
			</div>
			{#if scimToken}
				<div class="lp-raised grid gap-2 rounded-2xl border bg-card p-4 text-sm">
					<p class="font-medium">{t('organizations.sso.scim_token_once')}</p>
					<div class="grid gap-1 sm:grid-cols-[8rem_minmax(0,1fr)]">
						{@render infoLabel(t('organizations.sso.tenant_url'), 'tenant_url')}
						{@render copyable(scimToken.base_url)}
						{@render infoLabel(t('organizations.sso.secret_token'), 'secret_token')}
						{@render copyable(scimToken.token)}
					</div>
				</div>
			{/if}
			<div class="flex flex-wrap gap-2">
				<Button variant="outline" disabled={busy} onclick={rotateScim}>
					{settings.scim_enabled
						? t('organizations.sso.rotate_scim')
						: t('organizations.sso.enable_scim')}
				</Button>
				{#if settings.scim_enabled}
					<Button
						variant="ghost"
						class="text-destructive hover:text-destructive"
						disabled={busy}
						onclick={turnOffScim}>{t('organizations.sso.disable_scim')}</Button>
				{/if}
			</div>
		</section>

		<!-- Enforcement -->
		<section class="space-y-3" aria-labelledby="sso-enforce">
			<div>
				<h3 id="sso-enforce" class="text-[15px] font-semibold">{t('organizations.sso.enforcement')}</h3>
				<p class="mt-1 text-sm text-muted-foreground">{t('organizations.sso.enforcement_hint')}</p>
			</div>
			<label class="flex items-center gap-2 text-sm">
				<SoftCheckbox
					checked={settings.enforced}
					disabled={busy || (!settings.enforced && !anyEnabled)}
					onCheckedChange={(on) => toggleEnforcement(on === true)} />
				{t('organizations.sso.require_sso')}
			</label>
		</section>
	</div>
{/if}

<Dialog.Root bind:open={editorOpen}>
	<Dialog.Content class="flex flex-col gap-0 overflow-y-hidden p-0 sm:max-w-2xl">
		<Dialog.Header class="border-b p-6 pr-14">
			<Dialog.Title>
				{editing ? t('organizations.sso.edit_connection') : t('organizations.sso.new_connection')}
			</Dialog.Title>
			<Dialog.Description>{t('organizations.sso.editor_hint')}</Dialog.Description>
		</Dialog.Header>
		<div class="grid min-h-0 flex-1 gap-4 overflow-y-auto p-6">
			{#if !editing && settings.saml_supported}
				<div class="grid gap-2">
					{@render fieldLabel('sso-kind', t('organizations.sso.protocol'), 'protocol')}
					<NativeSelect
						id="sso-kind"
						value={form.kind}
						onchange={(e) => chooseKind(e.currentTarget.value as 'oidc' | 'saml')}
						class="w-full sm:w-64">
						<option value="oidc">OpenID Connect</option>
						<option value="saml">SAML 2.0</option>
					</NativeSelect>
				</div>
			{/if}
			{#if form.kind === 'oidc'}
				<details class="lp-inset rounded-xl bg-muted/40 p-3 text-sm">
					<summary class="cursor-pointer font-medium">{t('organizations.sso.entra_guide')}</summary>
					<ol class="mt-2 list-decimal space-y-1 ps-5 text-muted-foreground">
						<li>{t('organizations.sso.entra_step_1')}</li>
						<li>{t('organizations.sso.entra_step_2')}</li>
						<li>{t('organizations.sso.entra_step_3')}</li>
						<li>{t('organizations.sso.entra_step_4')}</li>
						<li>{t('organizations.sso.entra_step_5')}</li>
					</ol>
				</details>
			{/if}
			<div class="grid gap-1 text-sm">
				{#if form.kind === 'saml'}
					{@render infoLabel(t('organizations.sso.acs_url'), 'acs_url')}
				{:else}
					{@render infoLabel(t('organizations.sso.redirect_uri'), 'redirect_uri')}
				{/if}
				<!-- Before saving, the server value does not exist yet: derive it from the URL in use. -->
				{@render copyable(
					editing?.redirect_uri ??
						`${location.origin}/api/v1/auth/${form.kind === 'saml' ? 'saml/acs' : 'sso/callback'}`,
				)}
				{#if editing?.sp_entity_id}
					{@render infoLabel(t('organizations.sso.sp_entity_id'), 'sp_entity_id', 'mt-2')}
					{@render copyable(editing.sp_entity_id)}
				{/if}
				{#if editing?.post_logout_redirect_uri}
					{@render infoLabel(
						t('organizations.sso.post_logout_redirect_uri'),
						'post_logout_redirect_uri',
						'mt-2',
					)}
					{@render copyable(editing.post_logout_redirect_uri)}
				{/if}
				{#if editing?.backchannel_logout_url}
					{@render infoLabel(
						t('organizations.sso.backchannel_logout_url'),
						'backchannel_logout_url',
						'mt-2',
					)}
					{@render copyable(editing.backchannel_logout_url)}
				{/if}
			</div>
			<div class="grid gap-2">
				{@render fieldLabel('sso-name', t('organizations.sso.name'), 'name')}
				<Input id="sso-name" bind:value={form.name} placeholder="Microsoft Entra ID" />
			</div>
			{#if form.kind === 'saml'}
				<div class="grid gap-2">
					{@render fieldLabel('sso-metadata', t('organizations.sso.idp_metadata'), 'idp_metadata')}
					<textarea
						id="sso-metadata"
						bind:value={form.idp_metadata_xml}
						rows="6"
						class="min-h-32 w-full rounded-md border bg-transparent px-3 py-2 font-mono text-xs"
						placeholder={editing?.idp_entity_id
							? t('organizations.sso.metadata_keep', { entity: editing.idp_entity_id })
							: '<EntityDescriptor …>'}></textarea>
					<p class="text-xs text-muted-foreground">{t('organizations.sso.idp_metadata_hint')}</p>
				</div>
			{:else}
				<div class="grid gap-2">
					{@render fieldLabel('sso-issuer', t('organizations.sso.issuer'), 'issuer')}
					<Input
						id="sso-issuer"
						bind:value={form.issuer}
						placeholder="https://login.microsoftonline.com/<tenant-id>/v2.0" />
				</div>
				<div class="grid gap-4 sm:grid-cols-2">
					<div class="grid gap-2">
						{@render fieldLabel('sso-client', t('organizations.sso.client_id'), 'client_id')}
						<Input id="sso-client" bind:value={form.client_id} />
					</div>
					<div class="grid gap-2">
						{@render fieldLabel(
							'sso-secret',
							t('organizations.sso.client_secret'),
							'client_secret',
						)}
						<Input
							id="sso-secret"
							type="password"
							autocomplete="off"
							bind:value={form.client_secret}
							placeholder={editing?.has_client_secret ? t('organizations.sso.secret_keep') : ''} />
					</div>
				</div>
			{/if}
			<div class="grid gap-4 sm:grid-cols-2">
				<div class="grid gap-2">
					{@render fieldLabel('sso-groups', t('organizations.sso.groups_claim'), 'groups_claim')}
					<Input id="sso-groups" bind:value={form.groups_claim} />
				</div>
				<div class="grid gap-2">
					{@render fieldLabel('sso-role', t('organizations.sso.default_role'), 'default_role')}
					<NativeSelect
						id="sso-role"
						value={String(form.default_base_role_id)}
						onchange={(e) => (form.default_base_role_id = Number(e.currentTarget.value))}
						class="w-full">
						{#each BASE_ROLES as [id, name] (id)}
							<option value={String(id)}>{name}</option>
						{/each}
					</NativeSelect>
				</div>
			</div>
			<div class="grid gap-2">
				{@render fieldLabel('sso-ttl', t('organizations.sso.session_ttl'), 'session_ttl')}
				<Input
					id="sso-ttl"
					type="number"
					min="5"
					max="43200"
					step="5"
					bind:value={form.session_ttl_minutes} />
				<p class="text-xs text-muted-foreground">{t('organizations.sso.session_ttl_hint')}</p>
			</div>
			<div class="flex items-center gap-1.5">
				<label class="flex items-center gap-2 text-sm">
					<SoftCheckbox bind:checked={form.jit_enabled} />
					{t('organizations.sso.jit')}
				</label>
				<SsoFieldInfo field="jit" title={t('organizations.sso.jit')} />
			</div>
			<div class="flex items-center gap-1.5">
				<label class="flex items-center gap-2 text-sm">
					<SoftCheckbox bind:checked={form.enabled} />
					{t('organizations.sso.enable_connection')}
				</label>
				<SsoFieldInfo field="enabled" title={t('organizations.sso.enable_connection')} />
			</div>
		</div>
		<Dialog.Footer class="border-t p-4">
			<Button variant="outline" onclick={() => (editorOpen = false)}
				>{t('organizations.common.cancel')}</Button>
			<Button disabled={busy || !formComplete} onclick={saveEditor}
				>{t('organizations.sso.save')}</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>

<Dialog.Root
	open={mappingFor !== null}
	onOpenChange={(o) => {
		if (!o) mappingFor = null;
	}}>
	<Dialog.Content class="flex flex-col gap-0 overflow-y-hidden p-0 sm:max-w-2xl">
		<Dialog.Header class="border-b p-6 pr-14">
			<Dialog.Title>{t('organizations.sso.group_mappings')}</Dialog.Title>
			<Dialog.Description>{t('organizations.sso.mappings_hint')}</Dialog.Description>
		</Dialog.Header>
		<div class="grid min-h-0 flex-1 content-start gap-3 overflow-y-auto p-6">
			{#if mappings.length}
				<div
					class="hidden gap-2 text-sm sm:grid sm:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_2.25rem]">
					{@render infoLabel(t('organizations.sso.external_group'), 'external_group')}
					<span class="text-muted-foreground">{t('organizations.sso.litepod_group')}</span>
				</div>
			{/if}
			{#each mappings as mapping, index (index)}
				<div class="grid gap-2 sm:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_auto]">
					<Input
						bind:value={mapping.external_group}
						placeholder={t('organizations.sso.external_group')}
						aria-label={t('organizations.sso.external_group')} />
					<NativeSelect
						bind:value={mapping.group_id}
						class="w-full"
						aria-label={t('organizations.sso.litepod_group')}>
						<option value="" disabled>{t('organizations.sso.litepod_group')}</option>
						{#each mappable as group (group.id)}
							<option value={group.id}>{group.name}</option>
						{/each}
					</NativeSelect>
					<Button
						variant="ghost"
						size="icon"
						class="text-destructive hover:text-destructive"
						aria-label={t('organizations.sso.remove')}
						onclick={() => (mappings = mappings.filter((_, i) => i !== index))}
						><Trash2Icon class="size-4" /></Button>
				</div>
			{/each}
			{#if mappable.length === 0}
				<p class="text-sm text-muted-foreground">{t('organizations.sso.create_groups_first')}</p>
			{/if}
			<Button
				variant="outline"
				class="w-fit"
				onclick={() => (mappings = [...mappings, { external_group: '', group_id: '' }])}>
				<PlusIcon class="size-4" />{t('organizations.sso.add_mapping')}
			</Button>
		</div>
		<Dialog.Footer class="border-t p-4">
			<Button variant="outline" onclick={() => (mappingFor = null)}
				>{t('organizations.common.cancel')}</Button>
			<Button disabled={busy} onclick={saveMappings}>{t('organizations.sso.save')}</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>
