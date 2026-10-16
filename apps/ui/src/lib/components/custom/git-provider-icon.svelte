<script lang="ts">
	import type { Component } from 'svelte';
	import GitBranchIcon from '@lucide/svelte/icons/git-branch';
	import BitbucketIcon from '#lib/components/icons/bitbucket-icon.svelte';
	import ForgejoIcon from '#lib/components/icons/forgejo-icon.svelte';
	import GiteaIcon from '#lib/components/icons/gitea-icon.svelte';
	import GithubIcon from '#lib/components/icons/github-icon.svelte';
	import GitlabIcon from '#lib/components/icons/gitlab-icon.svelte';

	let { type, class: className = 'size-4' }: { type: string; class?: string } = $props();

	const ICONS: Record<string, Component<{ class?: string }>> = {
		github: GithubIcon,
		gitlab: GitlabIcon,
		bitbucket: BitbucketIcon,
		gitea: GiteaIcon,
		forgejo: ForgejoIcon,
	};

	let Icon = $derived(ICONS[type] ?? GitBranchIcon);
</script>

<Icon class={className} />
