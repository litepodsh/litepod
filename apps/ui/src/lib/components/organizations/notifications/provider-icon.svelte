<script lang="ts">
	import type { ProviderKind } from '#lib/clients/notification-client.js';
	import SlackIcon from '#lib/components/icons/slack-icon.svelte';
	import DiscordIcon from '#lib/components/icons/discord-icon.svelte';
	import TelegramIcon from '#lib/components/icons/telegram-icon.svelte';
	import MsTeamsIcon from '#lib/components/icons/msteams-icon.svelte';

	let { kind, class: className = 'size-6' }: { kind: ProviderKind; class?: string } = $props();

	const ICONS = {
		slack: SlackIcon,
		discord: DiscordIcon,
		telegram: TelegramIcon,
		msteams: MsTeamsIcon,
	} as const;

	const Icon = $derived(ICONS[kind]);
</script>

<Icon class={className} />
