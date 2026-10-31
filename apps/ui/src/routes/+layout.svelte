<script lang="ts">
	import '../app.css';
	import favicon from '#lib/assets/favicon.svg';
	import { page } from '$app/state';
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { Toaster } from '#lib/components/ui/sonner/index.js';
	import * as Tooltip from '#lib/components/ui/tooltip/index.js';
	import { ModeWatcher } from 'mode-watcher';
	import { onAuthMessage } from '#lib/auth-broadcast.js';
	import { restoreLocale } from '#lib/i18n/index.svelte.js';
	import { readStoredPalette, setPalette } from '#lib/state/palette.svelte.js';
	import { branding, brandedTitle, loadBranding } from '#lib/ee/branding/branding.svelte.js';

	let { children } = $props();

	setPalette(readStoredPalette());

	onMount(() => onAuthMessage(() => goto('/login')));
	onMount(restoreLocale);
	onMount(loadBranding);

	// A deploy replaces every content-hashed chunk filename. A tab left open across
	// a deploy will keep requesting old chunks that no longer exist, and the server
	// returns a failed fetch. Reload once to pick up the current build, but never
	// turn a persistent asset error into an infinite reload loop.
	onMount(() => {
		const retryKey = 'litepod:preload-error-retried';
		window.addEventListener('vite:preloadError', (event) => {
			event.preventDefault();
			if (sessionStorage.getItem(retryKey)) return;
			sessionStorage.setItem(retryKey, '1');
			window.location.reload();
		});
	});

	function pageTitle(): string {
		const path = page.url.pathname;
		if (path === '/login') return brandedTitle('Login', '-');
		if (path === '/register') return brandedTitle('Register', '-');
		if (path === '/verify-email') return brandedTitle('Verify email', '-');
		if (path === '/terms') return brandedTitle('Terms of Service', '-');
		if (path === '/privacy') return brandedTitle('Privacy Policy', '-');
		if (path === '/refund') return brandedTitle('Refund Policy', '-');
		if (path === '/acceptable-use') return brandedTitle('Acceptable Use Policy', '-');
		return brandedTitle();
	}
</script>

<svelte:head>
	<link rel="icon" href={branding.favicon_url ?? favicon} />
	<title>{pageTitle()}</title>
</svelte:head>

<ModeWatcher defaultMode="light" track={true} synchronousModeChanges />
<Toaster />
<!-- Sidebar.Provider used to supply this; every page's tooltips need it. -->
<Tooltip.Provider>
	{@render children()}
</Tooltip.Provider>
