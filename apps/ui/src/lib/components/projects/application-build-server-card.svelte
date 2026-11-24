<script lang="ts">
	import * as Card from '#lib/components/ui/card/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import SearchableDropdown from '#lib/components/custom/searchable-dropdown.svelte';
	import HammerIcon from '@lucide/svelte/icons/hammer';
	import * as Select from '#lib/components/ui/select/index.js';
	import {
		getBuildServer,
		saveBuildServer,
		type BuildDelivery,
	} from '#lib/clients/project-client.js';
	import {
		listRegistryCredentials,
		listServers,
		type RegistryCredential,
		type Server,
	} from '#lib/clients/server-client.js';
	import { toast, toastError } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		organizationId,
		projectId,
		environmentId,
		applicationId,
		canWrite,
		deployType,
		buildMethod,
		runsOn,
	}: {
		organizationId: string;
		projectId: string;
		environmentId: string;
		applicationId: string;
		canWrite: boolean;
		deployType: string;
		buildMethod: string;
		/** Where the application runs, which is also where it builds by default. */
		runsOn: string | null;
	} = $props();

	// Only a Git application builds; the others pull an image someone already built.
	const builds = $derived(deployType === 'git');

	const NONE = '';
	let servers = $state<Server[]>([]);
	let credentials = $state<RegistryCredential[]>([]);
	let serverId = $state<string | null>(NONE);
	let repository = $state('');
	let delivery = $state<BuildDelivery>('transfer');
	let saving = $state(false);

	// Only build-only servers build; they never run applications.
	const builders = $derived(servers.filter((server) => server.role === 'build'));
	const sameServer = $derived(
		runsOn
			? t('projects.build_server.none_on', { server: runsOn })
			: t('projects.build_server.none'),
	);
	const items = $derived([
		{ value: NONE, label: sameServer },
		...builders.map((server) => ({
			value: server.id,
			label: server.name,
			description: server.ssh_host ?? undefined,
		})),
	]);
	// Saved logins are keyed by lowercase host, as the API matches them.
	const registry = $derived((repository.trim().split('/')[0] ?? '').toLowerCase());
	const login = $derived(credentials.find((credential) => credential.registry === registry));

	$effect(() => {
		if (!builds) return;
		Promise.all([
			getBuildServer(organizationId, projectId, environmentId, applicationId),
			listServers(organizationId),
			listRegistryCredentials(organizationId),
		])
			.then(([settings, list, saved]) => {
				servers = list;
				credentials = saved;
				serverId = settings.build_server_id ?? NONE;
				repository = settings.image_repository ?? '';
				delivery = settings.delivery;
			})
			.catch((err) => toastError(err, t('projects.build_server.save_failed')));
	});

	async function save(event: SubmitEvent) {
		event.preventDefault();
		saving = true;
		try {
			const saved = await saveBuildServer(organizationId, projectId, environmentId, applicationId, {
				build_server_id: serverId || null,
				delivery,
				image_repository: repository.trim(),
			});
			repository = saved.image_repository ?? '';
			delivery = saved.delivery;
			toast.success(t('projects.build_server.saved'));
		} catch (err) {
			toastError(err, t('projects.build_server.save_failed'));
		} finally {
			saving = false;
		}
	}
</script>

<Card.Root class="max-w-[820px] overflow-hidden">
	<Card.Header>
		<div class="flex items-center gap-2">
			<HammerIcon class="size-4 text-muted-foreground" />
			<Card.Title>{t('projects.build_server.title')}</Card.Title>
		</div>
		<Card.Description>{t('projects.build_server.description')}</Card.Description>
	</Card.Header>
	<Card.Content class="pt-2">
		{#if !builds}
			<p class="text-sm text-muted-foreground">{t('projects.build_server.not_applicable')}</p>
		{:else}
			<form class="grid gap-4" onsubmit={save}>
				<div class="grid gap-2">
					<Label>{t('projects.build_server.server')}</Label>
					<SearchableDropdown
						{items}
						bind:selectedValue={serverId}
						label={sameServer}
						disabled={!canWrite || saving} />
					{#if builders.length === 0}
						<p class="text-xs text-muted-foreground">
							{t('projects.build_server.no_build_servers')}
						</p>
					{:else if buildMethod === 'compose'}
						<p class="text-xs text-muted-foreground">{t('projects.build_server.compose_hint')}</p>
					{/if}
				</div>
				{#if serverId}
					<div class="grid gap-2">
						<Label for="build-delivery">{t('projects.build_server.delivery')}</Label>
						<Select.Root type="single" bind:value={delivery} disabled={!canWrite || saving}>
							<Select.Trigger id="build-delivery" class="w-full"
								>{t(`projects.build_server.delivery_${delivery}`)}</Select.Trigger>
							<Select.Content>
								<Select.Item value="transfer"
									>{t('projects.build_server.delivery_transfer')}</Select.Item>
								<Select.Item value="registry"
									>{t('projects.build_server.delivery_registry')}</Select.Item>
							</Select.Content>
						</Select.Root>
						<p class="text-xs text-muted-foreground">
							{t(`projects.build_server.delivery_${delivery}_hint`)}
						</p>
					</div>
				{/if}
				{#if serverId && delivery === 'registry'}
					<div class="grid gap-2">
						<Label for="build-image-repository">{t('projects.build_server.repository')}</Label>
						<Input
							id="build-image-repository"
							bind:value={repository}
							placeholder="registry.example.com/team/my-app"
							autocomplete="off"
							spellcheck="false"
							disabled={!canWrite || saving} />
						<p class="text-xs text-muted-foreground">
							{t('projects.build_server.repository_hint', {
								repository: repository.trim() || 'registry.example.com/team/my-app',
							})}
						</p>
					</div>
					{#if registry.includes('.') || registry.includes(':') || registry === 'localhost'}
						<p class="text-xs text-muted-foreground">
							{login
								? t('projects.build_server.login_saved', { registry, username: login.username })
								: t('projects.build_server.login_missing', { registry })}
						</p>
					{/if}
				{/if}
				{#if canWrite}
					<div>
						<Button type="submit" size="sm" disabled={saving}
							>{t('projects.build_server.save')}</Button>
					</div>
				{/if}
			</form>
		{/if}
	</Card.Content>
</Card.Root>
