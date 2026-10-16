<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { listOrganizations } from '#lib/clients/organization-client.js';
	import { resolveSelectedOrganization } from '#lib/utils/selected-organization.js';

	// Git providers live under organization settings now. The OAuth and GitHub App
	// callbacks still land here, so their query is carried over for the panel.
	onMount(async () => {
		const organization = resolveSelectedOrganization(await listOrganizations().catch(() => []));
		if (!organization) return void goto('/organizations', { replace: true });
		const params = new URLSearchParams(window.location.search);
		params.set('tab', 'git-providers');
		void goto(`/organizations/${encodeURIComponent(organization.id)}?${params}`, {
			replace: true,
		});
	});
</script>
