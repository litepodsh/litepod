<script lang="ts">
	import { page } from '$app/state';
	import { t } from '#lib/i18n/index.svelte.js';
</script>

<div class="flex min-h-screen items-center justify-center bg-background p-8">
	<div class="mx-auto flex max-w-2xl flex-col items-center text-center">
		{#if page.status === 404}
			<pre
				class="max-w-full overflow-x-auto text-center font-mono text-[0.5rem] leading-tight text-foreground sm:text-xs md:text-sm">
██╗  ██╗ ██████╗ ██╗  ██╗
██║  ██║██╔═████╗██║  ██║
███████║██║██╔██║███████║
╚════██║████╔╝██║╚════██║
     ██║╚██████╔╝     ██║
     ╚═╝ ╚═════╝      ╚═╝

███╗   ██╗ ██████╗ ████████╗    ███████╗ ██████╗ ██╗   ██╗███╗   ██╗██████╗
████╗  ██║██╔═══██╗╚══██╔══╝    ██╔════╝██╔═══██╗██║   ██║████╗  ██║██╔══██╗
██╔██╗ ██║██║   ██║   ██║       █████╗  ██║   ██║██║   ██║██╔██╗ ██║██║  ██║
██║╚██╗██║██║   ██║   ██║       ██╔══╝  ██║   ██║██║   ██║██║╚██╗██║██║  ██║
██║ ╚████║╚██████╔╝   ██║       ██║     ╚██████╔╝╚██████╔╝██║ ╚████║██████╔╝
╚═╝  ╚═══╝ ╚═════╝    ╚═╝       ╚═╝      ╚═════╝  ╚═════╝ ╚═╝  ╚═══╝╚═════╝
    </pre>
		{:else}
			<p class="font-mono text-6xl font-semibold">{page.status}</p>
		{/if}

		<p class="mt-8 text-muted-foreground text-lg">
			{page.status === 404
				? t('auth.not_found.page_not_found')
				: (page.error?.message ?? t('auth.not_found.error'))}
		</p>

		<a
			href="/"
			class="mt-4 inline-block rounded-lg bg-primary px-6 py-2 text-sm font-medium text-primary-foreground transition-colors hover:bg-primary/90">
			{t('auth.not_found.go_home')}
		</a>
	</div>
</div>
