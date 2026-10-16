<script lang="ts">
	import type { ServerSummaryDto } from '#lib/clients/project-client.js';
	import OsIcon from '#lib/components/os-icon.svelte';
	import { osLabel } from '#lib/utils/os-label.js';

	let { server, class: className = '' }: { server: ServerSummaryDto; class?: string } = $props();

	const os = $derived(osLabel(server.distribution ?? server.os, server.os_version));
</script>

<!-- Where an application actually runs. Remote deployments are indistinguishable from local
     ones once a card is on screen, so every surface that lists applications names the host. -->
<span
	class="flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground {className}"
	title={os ? `${server.name} · ${os}` : server.name}>
	<OsIcon os={server.os} distribution={server.distribution} class="size-3.5 shrink-0" />
	<span class="truncate">{server.name}</span>
</span>
