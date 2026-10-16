<script lang="ts">
	import { goto } from '$app/navigation';
	import { onMount } from 'svelte';
	import { getMe } from '#lib/clients/auth-client.js';

	let { children } = $props();
	let isSysadmin = $state(false);

	onMount(async () => {
		try {
			isSysadmin = (await getMe()).is_sysadmin;
			if (!isSysadmin) await goto('/home');
		} catch {
			await goto('/login');
		}
	});
</script>

{#if isSysadmin}
	{@render children()}
{/if}
