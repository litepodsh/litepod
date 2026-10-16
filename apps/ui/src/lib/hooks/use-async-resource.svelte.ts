import { toastError } from '#lib/utils/toast.js';

/** Fetch-on-demand data with `loading` state and a shared error-toast on failure —
 *  the "onMount fetch, $state loading flag, try/catch/toast/finally" shape repeated
 *  across every tab component.
 *
 * ## Usage
 * ```svelte
 * <script lang="ts">
 *      import { onMount } from 'svelte';
 *      import { UseAsyncResource } from '#lib/hooks/use-async-resource.svelte.js';
 *
 *      const sessions = new UseAsyncResource(() => listSessions(), 'Failed to load sessions');
 *      onMount(() => sessions.load());
 * </script>
 *
 * {#if sessions.loading}
 *      Loading…
 * {:else}
 *      {sessions.data}
 * {/if}
 * ```
 */
export class UseAsyncResource<T> {
	#data = $state<T>();
	#loading = $state(true);

	constructor(
		private fetcher: () => Promise<T>,
		private fallback: string,
	) {}

	get data() {
		return this.#data;
	}

	set data(value: T | undefined) {
		this.#data = value;
	}

	get loading() {
		return this.#loading;
	}

	async load() {
		this.#loading = true;
		try {
			this.#data = await this.fetcher();
		} catch (err) {
			toastError(err, this.fallback);
		} finally {
			this.#loading = false;
		}
	}
}
