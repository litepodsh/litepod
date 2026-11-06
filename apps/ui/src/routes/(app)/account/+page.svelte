<script lang="ts">
	import { onDestroy } from 'svelte';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { tabIndexFromKeydown } from '#lib/utils/tab-shortcut.js';
	import {
		clearScopeHero,
		clearShellTabs,
		setScopeHero,
		setShellTabs,
	} from '#lib/state/shell.svelte.js';
	import SettingsSection from '#lib/components/projects/settings-section.svelte';
	import ProfileTab from '#lib/components/account/profile-tab.svelte';
	import SecurityTab from '#lib/components/account/security-tab.svelte';
	import SessionsTab from '#lib/components/account/sessions-tab.svelte';
	import TokensTab from '#lib/components/account/tokens-tab.svelte';
	import DeactivateAccountTab from '#lib/components/account/deactivate-account-tab.svelte';
	import GettingStartedTab from '#lib/components/account/getting-started-tab.svelte';
	import ThemePicker from '#lib/components/custom/theme-picker.svelte';
	import UserIcon from '@lucide/svelte/icons/user';
	import PaletteIcon from '@lucide/svelte/icons/palette';
	import ShieldCheckIcon from '@lucide/svelte/icons/shield-check';
	import MonitorSmartphoneIcon from '@lucide/svelte/icons/monitor-smartphone';
	import KeyRoundIcon from '@lucide/svelte/icons/key-round';
	import CompassIcon from '@lucide/svelte/icons/compass';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import { t } from '#lib/i18n/index.svelte.js';

	const tabs = $derived([
		{ value: 'profile', label: t('account.tabs.profile'), icon: UserIcon },
		{ value: 'appearance', label: t('account.tabs.appearance'), icon: PaletteIcon },
		{ value: 'security', label: t('account.tabs.security'), icon: ShieldCheckIcon },
		{ value: 'sessions', label: t('account.tabs.sessions'), icon: MonitorSmartphoneIcon },
		{ value: 'tokens', label: t('account.tabs.tokens'), icon: KeyRoundIcon },
		{ value: 'getting-started', label: t('account.tabs.getting_started'), icon: CompassIcon },
		{ value: 'danger', label: t('account.tabs.danger_zone'), icon: TriangleAlertIcon },
	]);

	function tabFromUrl() {
		const tab = page.url.searchParams.get('tab');
		return tab && tabs.some((item) => item.value === tab) ? tab : 'profile';
	}

	let activeTab = $state(tabFromUrl());

	function setTab(value: string) {
		activeTab = value;
		const url = new URL(page.url.href);
		url.searchParams.set('tab', value);
		goto(url, { shallow: true, replace: true });
	}

	function handleTabShortcut(event: KeyboardEvent) {
		const index = tabIndexFromKeydown(event, tabs.length);
		if (index === null) return;
		event.preventDefault();
		setTab(tabs[index].value);
	}

	// The shell draws the title card and the tab capsule.
	$effect(() => {
		setScopeHero({
			title: t('account.account.title'),
			avatar: heroAvatar,
			meta: heroMeta,
		});
		setShellTabs(tabs, activeTab, setTab);
	});
	onDestroy(() => {
		clearScopeHero();
		clearShellTabs();
	});
</script>

<svelte:window onkeydown={handleTabShortcut} />

{#snippet heroAvatar()}
	<span class="grid size-full place-items-center bg-muted"><UserIcon class="size-5" /></span>
{/snippet}

{#snippet heroMeta()}
	<span class="max-w-2xl">{t('account.account.description')}</span>
{/snippet}

<div class="flex w-full min-w-0 flex-col gap-6 px-4 py-6 sm:px-6">
	<div class="flex max-w-[820px] flex-col gap-4">
		{#if activeTab === 'profile'}
			<ProfileTab />
		{:else if activeTab === 'appearance'}
			<SettingsSection
				title={t('menu.appearance.title')}
				description={t('menu.appearance.description')}>
				<ThemePicker />
			</SettingsSection>
		{:else if activeTab === 'security'}
			<SecurityTab />
		{:else if activeTab === 'sessions'}
			<SessionsTab />
		{:else if activeTab === 'tokens'}
			<TokensTab />
		{:else if activeTab === 'getting-started'}
			<GettingStartedTab />
		{:else if activeTab === 'danger'}
			<DeactivateAccountTab />
		{/if}
	</div>
</div>
