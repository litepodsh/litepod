<script lang="ts">
	import { t } from '#lib/i18n/index.svelte.js';
	import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';
	import { updateState } from '#lib/state/update.svelte.js';
</script>

{#if updateState.applying}
	<div
		class="fixed inset-0 z-100 grid place-items-center bg-background/95 p-6 text-center"
		role="alert"
		aria-live="polite">
		<div class="w-full max-w-sm space-y-4">
			<LoaderCircleIcon class="mx-auto size-10 animate-spin text-primary" />
			{#if updateState.phase === 'preparing'}
				<p class="text-2xl font-semibold">{t('common.update.preparing')}</p>
				<p class="text-muted-foreground">{t('common.update.preparing_hint')}</p>
			{:else}
				<p class="text-2xl font-semibold">{t('common.update.waiting')}</p>
				<p class="text-muted-foreground">{t('common.update.waiting_hint')}</p>
			{/if}
		</div>
	</div>
{/if}
