<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { logout } from '#lib/clients/auth-client.js';
	import { broadcastLogout } from '#lib/auth-broadcast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	onMount(async () => {
		broadcastLogout();
		let redirectUrl: string | null = null;
		try {
			redirectUrl = await logout();
		} catch {
			// already logged out / session invalid — still redirect
		}
		// End the identity provider's session too; it sends the browser back to /login.
		if (redirectUrl) {
			window.location.href = redirectUrl;
			return;
		}
		goto('/');
	});
</script>

<p class="text-muted-foreground p-8 text-center">{t('auth.logout.logging_out')}</p>
