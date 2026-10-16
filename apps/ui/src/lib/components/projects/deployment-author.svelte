<script lang="ts">
	import type { DeploymentRunActorDto } from '#lib/clients/project-client.js';
	import * as Avatar from '#lib/components/ui/avatar/index.js';
	import { avatarUrl } from '#lib/utils/avatar.js';
	import { deploymentAuthor } from '#lib/utils/deployment-author.js';
	import SparklesIcon from '@lucide/svelte/icons/sparkles';

	let {
		actor = null,
		triggeredBy = null,
		commitAuthorName = null,
		commitAuthorEmail = null,
	}: {
		actor?: DeploymentRunActorDto | null;
		triggeredBy?: string | null;
		commitAuthorName?: string | null;
		commitAuthorEmail?: string | null;
	} = $props();

	let author = $derived(deploymentAuthor(actor, triggeredBy, commitAuthorName, commitAuthorEmail));
</script>

<span
	class="flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground/70 normal-case"
	title={author.label}>
	{#if author.seed}
		<Avatar.Root size="sm">
			<Avatar.Image
				src={avatarUrl(author.seed.toLocaleLowerCase())}
				alt={author.label.toLocaleLowerCase()} />
			<Avatar.Fallback>{author.initials.toLocaleLowerCase()}</Avatar.Fallback>
		</Avatar.Root>
	{:else if author.kind === 'assistant'}
		<!-- No avatar to show, and "ai assistant" alone reads as a username. -->
		<SparklesIcon class="text-primary size-3.5 shrink-0" aria-hidden="true" />
	{/if}
	<span class="truncate">{author.label.toLocaleLowerCase()}</span>
</span>
