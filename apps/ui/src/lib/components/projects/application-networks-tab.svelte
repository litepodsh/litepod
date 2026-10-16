<script lang="ts">
	import SkeletonRows from '#lib/components/custom/skeleton-rows.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import PencilIcon from '@lucide/svelte/icons/pencil';
	import { Switch } from '#lib/components/ui/switch/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import type {
		ApplicationNetworkOverviewDto,
		LinkedApplicationDto,
		ResourceNetworkLinkDto,
	} from '#lib/clients/project-client.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		overview,
		loading = false,
		error = null,
		onRefresh,
		isCompose = false,
		onMakePrivateNetworkExplicit,
		resourceLinks = [],
		resourceConsumers = [],
		resourceLinksLoading = false,
		isResource = false,
		onToggleResourceLink,
	}: {
		overview: ApplicationNetworkOverviewDto | null;
		loading?: boolean;
		error?: string | null;
		onRefresh: () => Promise<void>;
		isCompose?: boolean;
		onMakePrivateNetworkExplicit?: () => void;
		resourceLinks?: ResourceNetworkLinkDto[];
		resourceConsumers?: LinkedApplicationDto[];
		resourceLinksLoading?: boolean;
		isResource?: boolean;
		onToggleResourceLink?: (resourceId: string, linked: boolean) => Promise<void>;
	} = $props();

	// One in-flight toggle at a time per row, so a double click cannot fire two
	// requests that resolve out of order.
	let pending = $state<string | null>(null);

	async function toggle(link: ResourceNetworkLinkDto) {
		if (!onToggleResourceLink || pending) return;
		pending = link.resource_application_id;
		try {
			await onToggleResourceLink(link.resource_application_id, !link.linked);
		} finally {
			pending = null;
		}
	}
</script>

<Card.Root>
	<Card.Header>
		<div class="flex flex-wrap items-center justify-between gap-3">
			<div>
				<Card.Title>{t('projects.application_networks.title')}</Card.Title>
				<Card.Description>
					{t('projects.application_networks.description')}
				</Card.Description>
			</div>
			<div class="flex flex-wrap gap-2">
				{#if isCompose && onMakePrivateNetworkExplicit}
					<Button variant="outline" onclick={onMakePrivateNetworkExplicit}>
						<PencilIcon />{t('projects.application_networks.make_private_editable')}
					</Button>
				{/if}
				<Button variant="outline" disabled={loading} onclick={() => void onRefresh()}>
					<RefreshCwIcon class={loading ? 'animate-spin' : ''} />{t(
						'projects.application_general.refresh',
					)}
				</Button>
			</div>
		</div>
		{#if isCompose && onMakePrivateNetworkExplicit}
			<p class="text-xs text-muted-foreground">
				{t('projects.application_networks.private_hint_1')} <code>private</code>
				{t('projects.application_networks.private_hint_2')}
			</p>
		{/if}
	</Card.Header>
	<Card.Content class="space-y-5">
		{#if isResource}
			<div>
				<h3 class="mb-1 text-sm font-medium">
					{t('projects.application_networks.connected_applications')}
				</h3>
				{#if resourceLinksLoading && !resourceConsumers.length}
					<SkeletonRows
						rows={1}
						height="h-16"
						label={t('projects.application_networks.loading_connected')} />
				{:else if !resourceConsumers.length}
					<div
						class="flex items-start gap-2 rounded-lg border border-warning/40 bg-warning/10 p-3 text-sm">
						<TriangleAlertIcon class="mt-0.5 size-4 shrink-0 text-warning" />
						<p class="text-muted-foreground">
							{t('projects.application_networks.no_consumers')}
						</p>
					</div>
				{:else}
					<ul class="divide-y rounded-lg border text-sm">
						{#each resourceConsumers as consumer (consumer.application_id)}
							<li class="px-3 py-2">{consumer.name}</li>
						{/each}
					</ul>
				{/if}
			</div>
		{:else if onToggleResourceLink}
			<div>
				<h3 class="mb-1 text-sm font-medium">
					{t('projects.application_networks.database_networks')}
				</h3>
				<p class="mb-2 text-xs text-muted-foreground">
					{t('projects.application_networks.database_networks_hint')}
				</p>
				{#if resourceLinksLoading && !resourceLinks.length}
					<SkeletonRows
						rows={1}
						height="h-16"
						label={t('projects.application_networks.loading_database_resources')} />
				{:else if !resourceLinks.length}
					<p class="rounded-lg border border-dashed p-4 text-sm text-muted-foreground">
						{t('projects.application_networks.no_database_resources')}
					</p>
				{:else}
					<div class="divide-y rounded-lg border">
						{#each resourceLinks as link (link.resource_application_id)}
							<div class="flex items-center justify-between gap-3 px-3 py-2">
								<div class="min-w-0">
									<Label
										for={`resource-network-${link.resource_application_id}`}
										class="cursor-pointer">
										{link.name}
									</Label>
									<p class="truncate font-mono text-xs text-muted-foreground">
										{link.network_name}
									</p>
								</div>
								<Switch
									id={`resource-network-${link.resource_application_id}`}
									checked={link.linked}
									disabled={pending !== null}
									onCheckedChange={() => void toggle(link)} />
							</div>
						{/each}
					</div>
				{/if}
			</div>
		{/if}
		{#if error}
			<div
				class="rounded-lg border border-destructive/40 bg-destructive/10 p-4 text-sm text-destructive">
				{error}
			</div>
		{:else if loading && !overview}
			<SkeletonRows
				rows={1}
				height="h-40"
				label={t('projects.application_networks.loading_topology')} />
		{:else if !overview || (!overview.networks.length && !overview.containers.length)}
			<p class="rounded-lg border border-dashed p-6 text-sm text-muted-foreground">
				{t('projects.dialogs_common.no_deployed_containers')}
			</p>
		{:else}
			{#if overview.findings.length}
				<div class="space-y-2">
					{#each overview.findings as finding (finding.code)}
						<div
							class={finding.severity === 'warning'
								? 'rounded-lg border border-warning/40 bg-warning/10 p-3 text-sm'
								: 'rounded-lg border bg-muted/50 p-3 text-sm'}>
							<p class="font-medium">
								{finding.severity === 'warning'
									? t('projects.application_networks.warning')
									: t('projects.application_networks.info')}
							</p>
							<p class="mt-1 text-muted-foreground">{finding.message}</p>
						</div>
					{/each}
				</div>
			{/if}

			<div>
				<h3 class="mb-2 text-sm font-medium">{t('projects.application_networks.title')}</h3>
				<div class="overflow-x-auto rounded-lg border">
					<table class="w-full min-w-[620px] text-left text-sm">
						<thead class="border-b bg-muted/50 text-muted-foreground">
							<tr>
								<th class="px-3 py-2 font-medium">{t('projects.projects.name')}</th>
								<th class="px-3 py-2 font-medium">{t('projects.application_networks.driver')}</th>
								<th class="px-3 py-2 font-medium">{t('projects.application_networks.scope')}</th>
								<th class="px-3 py-2 font-medium"
									>{t('projects.application_networks.app_members')}</th>
								<th class="px-3 py-2 font-medium"
									>{t('projects.application_networks.other_members')}</th>
							</tr>
						</thead>
						<tbody>
							{#each overview.networks as network (network.name)}
								<tr class="border-b last:border-0">
									<td class="px-3 py-2 font-mono text-xs">{network.name}</td>
									<td class="px-3 py-2">{network.driver ?? '—'}</td>
									<td class="px-3 py-2 capitalize">{network.scope}</td>
									<td class="px-3 py-2">{network.application_member_count}</td>
									<td class="px-3 py-2">{network.external_member_count}</td>
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
			</div>

			<div>
				<h3 class="mb-2 text-sm font-medium">
					{t('projects.application_networks.application_containers')}
				</h3>
				<div class="grid gap-3 md:grid-cols-2">
					{#each overview.containers as container (container.name)}
						<article class="rounded-lg border p-3">
							<p class="truncate font-mono text-xs font-medium">{container.name}</p>
							<p class="mt-1 text-xs text-muted-foreground">
								{container.service
									? t('projects.application_networks.service_label', { service: container.service })
									: t('projects.application_volumes.application_container')}
							</p>
							{#if container.networks.length}
								<ul class="mt-3 space-y-2 text-xs">
									{#each container.networks as network (network.name)}
										<li class="rounded bg-muted/60 p-2">
											<p class="font-mono">{network.name}</p>
											<p class="mt-1 text-muted-foreground">
												{network.ip_address ?? t('projects.application_networks.no_ip')}
												{network.aliases.length ? ` · ${network.aliases.join(', ')}` : ''}
											</p>
										</li>
									{/each}
								</ul>
							{:else}
								<p class="mt-3 text-xs text-muted-foreground">
									{t('projects.application_networks.no_network_attachment')}
								</p>
							{/if}
						</article>
					{/each}
				</div>
			</div>
		{/if}
	</Card.Content>
</Card.Root>
