import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';

const getMock = vi.fn();
const updateMock = vi.fn();

vi.mock('#lib/clients/container-metrics-client.js', async (importActual) => ({
	...(await importActual<object>()),
	getMetricsRetention: (...args: unknown[]) => getMock(...args),
	updateMetricsRetention: (...args: unknown[]) => updateMock(...args),
}));
vi.mock('#lib/utils/toast.js', () => ({ toast: { success: vi.fn(), error: vi.fn() } }));

import MetricsRetentionSetting from './metrics-retention-setting.svelte';

afterEach(() => {
	cleanup();
	getMock.mockReset();
	updateMock.mockReset();
});

const retention = (days: number) => ({ retention_days: days, default_days: 90, max_days: 365 });

async function setup() {
	getMock.mockResolvedValue(retention(7));
	render(MetricsRetentionSetting, { organizationId: 'org-1' });
	const input = screen.getByLabelText('Metrics retention') as HTMLInputElement;
	await vi.waitFor(() => expect(input.value).toBe('7'));
	return input;
}

describe('MetricsRetentionSetting', () => {
	it('saves a value inside 1..365', async () => {
		const input = await setup();
		updateMock.mockResolvedValue(retention(30));
		await fireEvent.input(input, { target: { value: '30' } });
		await fireEvent.change(input);
		expect(updateMock).toHaveBeenCalledWith('org-1', 30);
	});

	it.each(['0', '366', '2.5'])('rejects %s without calling the API', async (value) => {
		const input = await setup();
		await fireEvent.input(input, { target: { value } });
		await fireEvent.change(input);
		expect(updateMock).not.toHaveBeenCalled();
		expect(screen.getByRole('alert').textContent).toMatch(/between 1 and 365/);
	});
});
