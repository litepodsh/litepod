import type { StatusTone } from './status-tone';

export type DeploymentLogTone = 'error' | 'warning' | 'success' | 'stage' | 'neutral';

const errorPattern =
	/\b(error|failed|fatal|exception|panic|abort(?:ed)?|non[- ]zero|exit code)\b|npm\s+err!|#\d+\s+error\b/i;
const warningPattern = /\b(warn(?:ing)?|deprecated|deprecation|notice)\b/i;
const successPattern = /\b(success(?:fully)?|completed|finished|done|ready|built)\b/i;
const stagePattern = /^==>|^#\d+\s+\[/;

export function deploymentLogTone(level: string, message: string): DeploymentLogTone {
	if (level === 'error' || errorPattern.test(message)) return 'error';
	if (warningPattern.test(message)) return 'warning';
	if (successPattern.test(message)) return 'success';
	if (stagePattern.test(message)) return 'stage';
	return 'neutral';
}

/** Log tones onto the app-wide status vocabulary in `status-tone.ts`. */
export function deploymentLogStatusTone(tone: DeploymentLogTone): StatusTone {
	switch (tone) {
		case 'error':
			return 'danger';
		case 'warning':
			return 'warn';
		case 'success':
			return 'ok';
		case 'stage':
			return 'info';
		case 'neutral':
			return 'neutral';
	}
}

/** The status token each log tone paints with, as a CSS variable reference. */
const RULER_VAR: Record<Exclude<DeploymentLogTone, 'neutral'>, string> = {
	error: '--destructive',
	warning: '--warning',
	success: '--success',
	stage: '--info',
};

/**
 * Monaco paints the overview ruler to a canvas, so it needs a resolved colour
 * rather than a class or a `var()`. Read the live token so the ruler follows
 * the theme instead of drifting to a hardcoded hex.
 */
function rulerColor(tone: Exclude<DeploymentLogTone, 'neutral'>): string {
	if (typeof window === 'undefined') return '';
	return getComputedStyle(document.documentElement).getPropertyValue(RULER_VAR[tone]).trim();
}

/**
 * Shared decoration for both log views. `monacoApi` is the lazily-imported
 * module, passed in because each viewer loads Monaco on its own schedule.
 */
export function deploymentLogDecoration(
	monacoApi: typeof import('monaco-editor'),
	lineNumber: number,
	tone: DeploymentLogTone,
): import('monaco-editor').editor.IModelDeltaDecoration {
	const range = new monacoApi.Range(lineNumber, 1, lineNumber, 1);
	if (tone === 'neutral') return { range, options: {} };
	return {
		range,
		options: {
			inlineClassName: `deployment-log-${tone}-text`,
			isWholeLine: true,
			overviewRuler: {
				color: rulerColor(tone),
				position: monacoApi.editor.OverviewRulerLane.Right,
			},
		},
	};
}
