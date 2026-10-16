export type ApplicationDeferredTab =
	| 'containers'
	| 'deployment'
	| 'domains'
	| 'environment'
	| 'networks'
	| 'map'
	| 'volumes';

type DeferredTabLoaders = Record<ApplicationDeferredTab, () => Promise<void>>;

export function createDeferredTabLoader(loaders: DeferredTabLoaders) {
	const loaded = new Set<ApplicationDeferredTab>();
	const pending = new Map<ApplicationDeferredTab, Promise<void>>();

	return {
		async load(tab: ApplicationDeferredTab) {
			if (loaded.has(tab)) return;
			const active = pending.get(tab);
			if (active) return active;
			const request = loaders[tab]()
				.then(() => {
					loaded.add(tab);
				})
				.finally(() => {
					if (pending.get(tab) === request) pending.delete(tab);
				});
			pending.set(tab, request);
			return request;
		},
		async refresh(tab: ApplicationDeferredTab) {
			await loaders[tab]();
			loaded.add(tab);
		},
		reset() {
			loaded.clear();
			pending.clear();
		},
	};
}
