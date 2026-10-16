<script lang="ts">
	import { goto } from '$app/navigation';
	import { onMount } from 'svelte';
	import { checkSetup, getMe } from '#lib/clients/auth-client.js';

	onMount(async () => {
		try {
			await getMe();
			goto('/home');
			return;
		} catch {
			// no valid session, fall through to setup/login check
		}

		try {
			const setup = await checkSetup();
			if (setup.setup_required) {
				goto('/register');
			} else {
				goto('/login');
			}
		} catch {
			goto('/login');
		}
	});
</script>
