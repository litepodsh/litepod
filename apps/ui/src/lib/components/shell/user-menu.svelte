<script lang="ts">
	import { goto } from '$app/navigation';
	import * as Avatar from '#lib/components/ui/avatar/index.js';
	import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import BadgeCheckIcon from '@lucide/svelte/icons/badge-check';
	import CheckIcon from '@lucide/svelte/icons/check';
	import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';
	import CloudIcon from '@lucide/svelte/icons/cloud';
	import MessageSquareWarningIcon from '@lucide/svelte/icons/message-square-warning';
	import LogOutIcon from '@lucide/svelte/icons/log-out';
	import PaletteIcon from '@lucide/svelte/icons/palette';
	import ShieldCheckIcon from '@lucide/svelte/icons/shield-check';
	import LocaleFlag from '#lib/components/shell/locale-flag.svelte';
	import ThemePicker from '#lib/components/custom/theme-picker.svelte';
	import { i18n, setLocale, t } from '#lib/i18n/index.svelte.js';
	import { avatarUrl, initials } from '#lib/utils/avatar.js';
	import { cloudVersionLabel } from '#lib/utils/cloud-version-label.js';
	import type { SystemVersion } from '#lib/clients/system-client.js';
	import { cn } from '#lib/utils.js';

	// Personal things only: organization and instance pages live in the scope tabs.
	let {
		user,
		systemVersion,
	}: {
		user: { name: string; email: string; is_sysadmin: boolean };
		systemVersion: SystemVersion | null;
	} = $props();

	// Phone-only collapsibles; both start folded every time the menu opens.
	const ISSUES_URL = 'https://github.com/litepodsh/litepod/issues';

	let appearanceOpen = $state(false);
	let languageOpen = $state(false);

	// Admin is the only instance page without a scope tab of its own.
	let showAdmin = $derived(user.is_sysadmin && (systemVersion?.is_cloud ?? false));
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
		{#if showAdmin}
			<DropdownMenu.Item onclick={() => goto('/admin')}>
				<ShieldCheckIcon />
				{t('menu.navigation.admin')}
			</DropdownMenu.Item>
			<DropdownMenu.Separator />
		{/if}
		<DropdownMenu.Item onclick={() => window.open(ISSUES_URL, '_blank', 'noopener,noreferrer')}>
			<MessageSquareWarningIcon />
			{t('menu.user_menu.feedback')}
		</DropdownMenu.Item>
		<DropdownMenu.Item onclick={() => goto('/logout')}>
			<LogOutIcon />
			{t('menu.user_menu.log_out')}
		</DropdownMenu.Item>
	</DropdownMenu.Content>
</DropdownMenu.Root>
