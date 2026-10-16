<script lang="ts">
	import { t } from '#lib/i18n/index.svelte.js';
	import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';
	import { Progress } from '#lib/components/ui/progress/index.js';
	import { GRACE_SECONDS, updateState } from '#lib/state/update.svelte.js';
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
			{:else if updateState.phase === 'grace'}
				<p class="text-2xl font-semibold">{t('common.update.applying')}</p>
				<p class="text-muted-foreground">
					{t('common.update.checking_in')}
					<span class="font-medium text-foreground tabular-nums">{updateState.secondsLeft}s</span>
				</p>
				<Progress value={GRACE_SECONDS - updateState.secondsLeft} max={GRACE_SECONDS} />
			{:else}
				<p class="text-2xl font-semibold">{t('common.update.waiting')}</p>
			{/if}
		</div>
	</div>
{/if}
