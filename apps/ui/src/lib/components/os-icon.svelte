<script lang="ts">
	import ServerIcon from '@lucide/svelte/icons/server';
	import AppleIcon from '#lib/components/icons/apple-icon.svelte';
	import LinuxIcon from '#lib/components/icons/linux-icon.svelte';
	import DebianIcon from '#lib/components/icons/debian-icon.svelte';
	import UbuntuIcon from '#lib/components/icons/ubuntu-icon.svelte';
	import FedoraIcon from '#lib/components/icons/fedora-icon.svelte';
	import ArchIcon from '#lib/components/icons/arch-icon.svelte';
	import OmarchyIcon from '#lib/components/icons/omarchy-icon.svelte';
	import RedHatIcon from '#lib/components/icons/red-hat-icon.svelte';
	import RockyLinuxIcon from '#lib/components/icons/rocky-linux-icon.svelte';

	let {
		os,
		distribution,
		class: className = 'size-3.5',
	}: { os?: string | null; distribution?: string | null; class?: string } = $props();
</script>

<!-- Remote servers only report a distribution id, so a known distro stands on its own
     without the caller also having to say it is Linux. -->
{#if os === 'macos'}
	<AppleIcon class={className} />
{:else if distribution === 'debian'}
	<DebianIcon class={className} />
{:else if distribution === 'ubuntu'}
	<UbuntuIcon class={className} />
{:else if distribution === 'fedora'}
	<FedoraIcon class={className} />
{:else if distribution === 'arch'}
	<ArchIcon class={className} />
{:else if distribution === 'omarchy'}
	<OmarchyIcon class={className} />
{:else if distribution === 'rhel'}
	<RedHatIcon class={className} />
{:else if distribution === 'rocky'}
	<RockyLinuxIcon class={className} />
{:else if os === 'linux' || distribution}
	<LinuxIcon class={className} />
{:else}
	<ServerIcon class={className} aria-hidden="true" />
{/if}
