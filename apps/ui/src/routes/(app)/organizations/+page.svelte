<script lang="ts">
	import { onMount } from 'svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import { listOrganizations, type OrganizationDto } from '#lib/clients/organization-client.js';
	import { toast } from '#lib/utils/toast.js';
	import OrganizationIcon from '#lib/components/organization-icon.svelte';
	import Building2Icon from '@lucide/svelte/icons/building-2';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import ArrowUpRightIcon from '@lucide/svelte/icons/arrow-up-right';
	import { t } from '#lib/i18n/index.svelte.js';

	let organizations = $state<OrganizationDto[]>([]);
	let loading = $state(true);

	onMount(async () => {
		try {
			organizations = await listOrganizations();
		} catch (err) {
			toast.error(err instanceof Error ? err.message : t('organizations.list.load_failed'));
		} finally {
			loading = false;
		}
	});
</script>

<svelte:head>
	<title>{t('organizations.list.title')} · litepod</title>
</svelte:head>

<div class="flex flex-col gap-5 px-4 py-6 sm:px-6">
	<div class="flex flex-wrap items-end justify-between gap-4">
		<div class="min-w-0">
			<h1 class="text-xl font-semibold tracking-tight">{t('organizations.list.title')}</h1>
			<p class="text-sm text-muted-foreground">{t('organizations.list.description')}</p>
		</div>
		<Button href="/organizations/add" class="max-sm:w-full">
			<PlusIcon class="size-4" />
			{t('organizations.list.add')}
		</Button>
	</div>

	{#if loading}
		<div
			class="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3"
			role="status"
			aria-busy="true"
			aria-label={t('organizations.list.loading')}>
			{#each { length: 3 } as _, index (index)}
				<Skeleton class="h-28 rounded-2xl" />
			{/each}
		</div>
	{:else if organizations.length === 0}
		<div
			class="flex min-h-64 flex-col items-center justify-center rounded-2xl border border-dashed border-foreground/15 bg-card p-8 text-center">
			<div class="lp-raised flex size-12 items-center justify-center rounded-xl border bg-muted">
				<Building2Icon class="size-5 text-muted-foreground" />
			</div>
			<h2 class="mt-4 font-medium">{t('organizations.list.empty')}</h2>
			<p class="mt-1 max-w-sm text-sm text-muted-foreground">{t('organizations.list.empty_hint')}</p>
			<Button class="mt-5" href="/organizations/add">
				<PlusIcon class="size-4" />
				{t('organizations.list.add')}
			</Button>
		</div>
	{:else}
		<section
			aria-label={t('organizations.list.title')}
			class="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3">
			{#each organizations as organization (organization.id)}
				<a
					href="/organizations/{organization.id}"
					class="group lp-raised flex flex-col gap-3.5 rounded-2xl border bg-card p-[18px] transition-[border-color,translate] duration-300 hover:-translate-y-0.5 hover:border-primary/35 motion-reduce:transition-none motion-reduce:hover:translate-y-0">
					<header class="flex items-start gap-3">
						<span
							class="lp-inset grid size-9 shrink-0 place-items-center overflow-hidden rounded-[9px] border {organization.icon_bg
								? ''
								: 'bg-muted'}">
							<OrganizationIcon icon={organization.icon} bg={organization.icon_bg} class="size-4" />
						</span>
						<span class="min-w-0 flex-1">
							<span class="block truncate text-[14.5px] font-semibold tracking-tight"
								>{organization.name}</span>
							<span class="block truncate font-mono text-[12px] text-muted-foreground"
								>{organization.slug}</span>
						</span>
						<ArrowUpRightIcon
							class="size-4 shrink-0 text-muted-foreground transition-transform group-hover:translate-x-0.5 group-hover:-translate-y-0.5" />
					</header>
					<footer class="lp-groove-t mt-auto flex flex-wrap gap-1.5 pt-3.5">
						{#if organization.plan}
							<span
								class="lp-raised inline-flex h-6 max-w-full items-center truncate rounded-md bg-muted px-2 text-xs text-muted-foreground"
								>{organization.plan}</span>
						{/if}
						{#if organization.is_root}
							<span
								class="lp-raised inline-flex h-6 items-center rounded-md bg-primary/10 px-2 text-xs font-medium text-primary"
								>{t('organizations.common.root')}</span>
						{/if}
					</footer>
				</a>
			{/each}
		</section>
	{/if}
</div>
