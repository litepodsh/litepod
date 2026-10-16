import {
	applyOnboardingAction,
	getOnboarding,
	type OnboardingAction,
	type OnboardingSummary,
} from '#lib/clients/onboarding-client.js';

export const ONBOARDING_INVALIDATE_EVENT = 'litepod:onboarding-invalidated';

export const onboardingState = $state<{
	organizationId: string | null;
	summary: OnboardingSummary | null;
	loading: boolean;
	stale: boolean;
	error: string;
	expanded: boolean;
}>({
	organizationId: null,
	summary: null,
	loading: false,
	stale: false,
	error: '',
	expanded: false,
});

let requestVersion = 0;
let refreshQueued = false;

export async function loadOnboarding(organizationId: string | null, retain = false) {
	const version = ++requestVersion;
	onboardingState.organizationId = organizationId;
	if (!retain) onboardingState.summary = null;
	onboardingState.error = '';
	onboardingState.stale = false;
	if (!organizationId) return;
	onboardingState.loading = true;
	try {
		const summary = await getOnboarding(organizationId);
		if (version !== requestVersion || onboardingState.organizationId !== organizationId) return;
		onboardingState.summary = summary;
	} catch (error) {
		if (version !== requestVersion) return;
		onboardingState.error =
			error instanceof Error ? error.message : 'Could not load getting started.';
		onboardingState.stale = !!onboardingState.summary;
	} finally {
		if (version === requestVersion) onboardingState.loading = false;
	}
}

export async function runOnboardingAction(action: OnboardingAction) {
	const organizationId = onboardingState.organizationId;
	if (!organizationId) return;
	onboardingState.error = '';
	try {
		onboardingState.summary = await applyOnboardingAction(organizationId, action);
		onboardingState.stale = false;
	} catch (error) {
		onboardingState.error =
			error instanceof Error ? error.message : 'Could not save this preference.';
		throw error;
	}
}

export function invalidateOnboarding() {
	if (typeof window !== 'undefined') window.dispatchEvent(new Event(ONBOARDING_INVALIDATE_EVENT));
}

export function installOnboardingInvalidation() {
	const refresh = () => {
		if (onboardingState.loading) {
			refreshQueued = true;
			return;
		}
		void loadOnboarding(onboardingState.organizationId, true).finally(() => {
			if (!refreshQueued) return;
			refreshQueued = false;
			void loadOnboarding(onboardingState.organizationId, true);
		});
	};
	window.addEventListener(ONBOARDING_INVALIDATE_EVENT, refresh);
	return () => window.removeEventListener(ONBOARDING_INVALIDATE_EVENT, refresh);
}
