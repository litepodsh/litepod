import { api } from './base-client';

export type GuideKey = 'deploy' | 'notifications';
export type DeployRoute = 'git' | 'static';
export type PanelPosition = 'right' | 'sidebar' | 'hidden';
export type TaskStatus = 'pending' | 'detected' | 'manual' | 'unavailable' | 'skipped';

export type OnboardingTask = {
	key: string;
	title: string;
	description: string;
	status: TaskStatus;
	completion_source: 'auto' | 'manual' | null;
	href: string | null;
	can_mark_done: boolean;
	blocked_reason: string | null;
};

export type OnboardingGuide = {
	key: GuideKey;
	title: string;
	completed: number;
	total: number;
	is_complete: boolean;
	next_task_key: string | null;
	tasks: OnboardingTask[];
};

export type OnboardingSummary = {
	active_guide: GuideKey;
	deploy_route: DeployRoute | null;
	panel_position: PanelPosition;
	dismissed: boolean;
	guides: OnboardingGuide[];
};

export type OnboardingAction =
	| { action: 'select_guide'; guide: GuideKey }
	| { action: 'select_route'; route: DeployRoute }
	| { action: 'set_position'; position: PanelPosition }
	| { action: 'set_guided_resource'; project_id?: string; application_id?: string }
	| { action: 'mark_task'; task: string }
	| { action: 'reopen_task'; task: string }
	| { action: 'dismiss' }
	| { action: 'reset' };

const path = (organizationId: string) =>
	`/api/v1/organizations/${encodeURIComponent(organizationId)}/onboarding`;

export async function getOnboarding(organizationId: string): Promise<OnboardingSummary> {
	return (await api<OnboardingSummary>(path(organizationId))).data!;
}

export async function applyOnboardingAction(
	organizationId: string,
	action: OnboardingAction,
): Promise<OnboardingSummary> {
	return (
		await api<OnboardingSummary>(path(organizationId), {
			method: 'PATCH',
			body: JSON.stringify(action),
		})
	).data!;
}
