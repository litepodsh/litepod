<script lang="ts">
	import { goto } from '$app/navigation';
	import type { Component } from 'svelte';
	import * as Avatar from '#lib/components/ui/avatar/index.js';
	import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import BadgeCheckIcon from '@lucide/svelte/icons/badge-check';
	import BellIcon from '@lucide/svelte/icons/bell';
	import BoxesIcon from '@lucide/svelte/icons/boxes';
	import BugIcon from '@lucide/svelte/icons/bug';
	import CheckIcon from '@lucide/svelte/icons/check';
	import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';
	import CloudIcon from '@lucide/svelte/icons/cloud';
	import ContainerIcon from '@lucide/svelte/icons/container';
	import CreditCardIcon from '@lucide/svelte/icons/credit-card';
	import DatabaseBackupIcon from '@lucide/svelte/icons/database-backup';
	import GitBranchIcon from '@lucide/svelte/icons/git-branch';
	import LogOutIcon from '@lucide/svelte/icons/log-out';
	import NetworkIcon from '@lucide/svelte/icons/network';
	import PaletteIcon from '@lucide/svelte/icons/palette';
	import ScrollTextIcon from '@lucide/svelte/icons/scroll-text';
	import ServerCogIcon from '@lucide/svelte/icons/server-cog';
	import SettingsIcon from '@lucide/svelte/icons/settings';
	import ShieldCheckIcon from '@lucide/svelte/icons/shield-check';
	import WaypointsIcon from '@lucide/svelte/icons/waypoints';
	import LocaleFlag from '#lib/components/shell/locale-flag.svelte';
	import ThemePicker from '#lib/components/custom/theme-picker.svelte';
	import { i18n, setLocale, t } from '#lib/i18n/index.svelte.js';
	import { avatarUrl, initials } from '#lib/utils/avatar.js';
	import { cloudVersionLabel } from '#lib/utils/cloud-version-label.js';
	import type { SystemVersion } from '#lib/clients/system-client.js';
	import { cn } from '#lib/utils.js';

	// Everything the old sidebar listed that is not a scope tab lives here.
	let {
		user,
		organizationId,
		systemVersion,
	}: {
		user: { name: string; email: string; is_sysadmin: boolean };
		organizationId: string | null;
		systemVersion: SystemVersion | null;
	} = $props();

	type Item = { title: string; url: string; icon: Component };

	// Phone-only collapsibles; both start folded every time the menu opens.
	let appearanceOpen = $state(false);
	let languageOpen = $state(false);

	let isCloud = $derived(systemVersion?.is_cloud ?? false);

	let organizationItems = $derived.by((): Item[] => {
		if (!organizationId) return [];
		const base = `/organizations/${encodeURIComponent(organizationId)}`;
		return [
			{
				title: t('menu.navigation.git_providers'),
				url: `${base}?tab=git-providers`,
				icon: GitBranchIcon,
			},
			{
				title: t('menu.navigation.notifications'),
				url: `${base}?tab=notifications`,
				icon: BellIcon,
			},
			{
				title: t('menu.navigation.audit_logs'),
				url: `${base}?tab=audit-logs`,
				icon: ScrollTextIcon,
			},
			{ title: t('menu.navigation.server_groups'), url: `${base}/server-groups`, icon: BoxesIcon },
			{ title: t('menu.navigation.map'), url: `${base}/topology`, icon: WaypointsIcon },
		];
	});

	// Same rules the sidebar applied: cloud members get Subscription, sysadmins the
	// instance pages, and self-hosted members keep instance Settings.
	let systemItems = $derived.by((): Item[] => {
		if (!user.is_sysadmin) {
			return isCloud
				? [{ title: t('menu.navigation.subscription'), url: '/billing', icon: CreditCardIcon }]
				: [{ title: t('menu.user_menu.settings'), url: '/settings', icon: SettingsIcon }];
		}
		return [
			...(isCloud
				? [{ title: t('menu.navigation.admin'), url: '/admin', icon: ShieldCheckIcon }]
				: []),
			{ title: t('menu.user_menu.settings'), url: '/settings', icon: SettingsIcon },
			{ title: t('menu.navigation.system'), url: '/system', icon: ServerCogIcon },
			{
				title: t('menu.navigation.containers'),
				url: '/system?tab=containers',
				icon: ContainerIcon,
			},
			{ title: t('menu.navigation.caddy'), url: '/system?tab=caddy', icon: NetworkIcon },
			{
				title: t('menu.user_menu.maintenance'),
				url: '/system?tab=maintenance',
				icon: ServerCogIcon,
			},
			{ title: t('menu.user_menu.debug'), url: '/system?tab=debug', icon: BugIcon },
			{
				title: t('menu.navigation.backups'),
				url: '/system?tab=backups',
				icon: DatabaseBackupIcon,
			},
		];
	});
</script>

{#snippet avatar(size: string)}
	<Avatar.Root class="{size} rounded-full">
		<Avatar.Image src={avatarUrl(user.name)} alt={`${user.name}'s avatar`} />
		<Avatar.Fallback class="rounded-full text-[11px]">{initials(user.name)}</Avatar.Fallback>
	</Avatar.Root>
{/snippet}

{#snippet languages()}
	<DropdownMenu.Item onclick={() => setLocale('en')}>
		<LocaleFlag locale="en" european={i18n.europeanFlags} />
		{t('menu.appearance.english')}
		{#if i18n.locale === 'en'}<CheckIcon class="ms-auto" />{/if}
	</DropdownMenu.Item>
	<DropdownMenu.Item onclick={() => setLocale('es')}>
		<LocaleFlag locale="es" european={i18n.europeanFlags} />
		{t('menu.appearance.spanish')}
		{#if i18n.locale === 'es'}<CheckIcon class="ms-auto" />{/if}
	</DropdownMenu.Item>
{/snippet}

{#snippet group(label: string, items: Item[])}
	{#if items.length}
		<DropdownMenu.Group>
			<DropdownMenu.Label class="text-xs text-muted-foreground">{label}</DropdownMenu.Label>
			{#each items as item (item.url)}
				<DropdownMenu.Item onclick={() => goto(item.url)}>
					<item.icon />
					{item.title}
				</DropdownMenu.Item>
			{/each}
		</DropdownMenu.Group>
		<DropdownMenu.Separator />
	{/if}
{/snippet}

<DropdownMenu.Root
	onOpenChange={(open) => {
		if (open) appearanceOpen = languageOpen = false;
	}}>
	<DropdownMenu.Trigger
		aria-label={user.name || t('menu.user_menu.account')}
		class="grid size-8 shrink-0 place-items-center rounded-full outline-none ring-offset-2 ring-offset-background transition-shadow focus-visible:ring-2 focus-visible:ring-ring data-[state=open]:ring-2 data-[state=open]:ring-foreground/15">
		{@render avatar('size-7.5')}
	</DropdownMenu.Trigger>
	<DropdownMenu.Content
		class="max-h-(--bits-dropdown-menu-content-available-height) w-64 rounded-xl max-sm:w-[min(20rem,calc(100vw-1.5rem))]"
		align="end"
		sideOffset={8}>
		<DropdownMenu.Label class="p-0 font-normal">
			<div class="flex items-center gap-2 px-1 py-1.5 text-start text-sm">
				{@render avatar('size-8')}
				<div class="grid flex-1 leading-tight">
					<span class="flex items-center gap-1.5">
						<span class="truncate font-medium">{user.name}</span>
						<Badge
							variant={user.is_sysadmin ? 'default' : 'secondary'}
							class="h-2.5 shrink-0 px-1 py-0 text-[8px] leading-none">
							{user.is_sysadmin ? 'Sysadmin' : 'Member'}
						</Badge>
					</span>
					<span class="truncate text-xs text-muted-foreground">{user.email}</span>
					{#if systemVersion}
						<span
							class="flex items-center gap-1 truncate font-mono text-[10px] text-muted-foreground"
							title={cloudVersionLabel(systemVersion.version, systemVersion.is_cloud)}>
							{systemVersion.version}
							{#if systemVersion.is_cloud}<CloudIcon
									class="size-3"
									aria-label="Cloud edition" />{/if}
						</span>
					{/if}
				</div>
			</div>
		</DropdownMenu.Label>
		<DropdownMenu.Separator />
		<DropdownMenu.Item onclick={() => goto('/account')}>
			<BadgeCheckIcon />
			{t('menu.user_menu.account')}
		</DropdownMenu.Item>
		<DropdownMenu.Sub>
			<DropdownMenu.SubTrigger class="max-sm:hidden" aria-label={t('menu.appearance.language')}>
				<LocaleFlag locale={i18n.locale} european={i18n.europeanFlags} />
				{t('menu.appearance.language')}
			</DropdownMenu.SubTrigger>
			<DropdownMenu.SubContent>{@render languages()}</DropdownMenu.SubContent>
		</DropdownMenu.Sub>
		<DropdownMenu.Item
			class="sm:hidden"
			aria-expanded={languageOpen}
			closeOnSelect={false}
			onSelect={() => (languageOpen = !languageOpen)}>
			<LocaleFlag locale={i18n.locale} european={i18n.europeanFlags} />
			{t('menu.appearance.language')}
			<ChevronDownIcon class={cn('ms-auto transition-transform', languageOpen && 'rotate-180')} />
		</DropdownMenu.Item>
		{#if languageOpen}
			<div class="ps-4 sm:hidden">{@render languages()}</div>
		{/if}
		<!-- A side submenu would run off a phone screen, so phones get the picker inline. -->
		<DropdownMenu.Sub>
			<DropdownMenu.SubTrigger class="max-sm:hidden">
				<PaletteIcon />
				{t('menu.appearance.title')}
			</DropdownMenu.SubTrigger>
			<DropdownMenu.SubContent class="w-80 max-w-[calc(100vw-2rem)] rounded-2xl p-3">
				<ThemePicker />
			</DropdownMenu.SubContent>
		</DropdownMenu.Sub>
		<DropdownMenu.Item
			class="sm:hidden"
			aria-expanded={appearanceOpen}
			closeOnSelect={false}
			onSelect={() => (appearanceOpen = !appearanceOpen)}>
			<PaletteIcon />
			{t('menu.appearance.title')}
			<ChevronDownIcon class={cn('ms-auto transition-transform', appearanceOpen && 'rotate-180')} />
		</DropdownMenu.Item>
		{#if appearanceOpen}
			<div class="px-1 pt-1 pb-1.5 sm:hidden">
				<ThemePicker />
			</div>
		{/if}
		<DropdownMenu.Separator />
		{@render group(t('menu.user_menu.organization'), organizationItems)}
		{@render group(t('menu.user_menu.system'), systemItems)}
		<DropdownMenu.Item onclick={() => goto('/logout')}>
			<LogOutIcon />
			{t('menu.user_menu.log_out')}
		</DropdownMenu.Item>
	</DropdownMenu.Content>
</DropdownMenu.Root>
