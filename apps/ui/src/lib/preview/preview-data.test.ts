import { expect, it } from 'vitest';
import { deploymentSteps } from './preview-data';

it('names compose and git steps', () => {
	expect(deploymentSteps('compose')).toEqual(['Prepare', 'Pull', 'Start', 'Health', 'Route']);
	expect(deploymentSteps('git')[0]).toBe('Clone');
});
