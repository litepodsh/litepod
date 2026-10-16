<script lang="ts">
	import { goto } from '$app/navigation';
	import { onMount } from 'svelte';
	import { getMe } from '#lib/clients/auth-client.js';
	import { getSystemVersion } from '#lib/clients/system-client.js';

	let { children } = $props();
	let allowed = $state(false);

	onMount(async () => {
		try {
			const [me, system] = await Promise.all([getMe(), getSystemVersion()]);
			allowed = me.is_sysadmin && system.is_cloud;
			if (!allowed) await goto('/home');
		} catch {
			await goto('/login');
		}
	});
</script>

{#if allowed}
	{@render children()}
{/if}
