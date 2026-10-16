<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { listOrganizations } from '#lib/clients/organization-client.js';
	import { resolveSelectedOrganization } from '#lib/utils/selected-organization.js';

	// Notifications live under organization settings now; old links land here.
	onMount(async () => {
		const organization = resolveSelectedOrganization(await listOrganizations().catch(() => []));
		if (!organization) return void goto('/organizations', { replace: true });
		void goto(`/organizations/${encodeURIComponent(organization.id)}?tab=notifications`, {
			replace: true,
		});
	});
</script>
