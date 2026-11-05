import { describe, expect, it } from 'vitest';

import type { ServerSecurity } from '#lib/clients/server-client.js';
import { firewallCommands, securityStepStatus } from './server-security';

const report: ServerSecurity = {
	password_login: false,
	root_login: 'without-password',
	ssh_level: 'recommended',
	firewall: 'ufw',
	firewall_active: false,
	fail2ban: 'active',
	banned_ips: 0,
	auto_updates: null,
	security_updates: null,
	reboot_required: false,
	public_ports: [22],
	ssh_settings: null,
};

describe('server security steps', () => {
	it('marks each step from the audit', () => {
		expect(securityStepStatus(report)).toEqual({
			ssh: 'done',
			fail2ban: 'done',
			firewall: 'todo',
			updates: 'unknown',
		});
		expect(securityStepStatus({ ...report, root_login: 'yes' }).ssh).toBe('todo');
		expect(securityStepStatus({ ...report, password_login: null }).ssh).toBe('unknown');
	});

	it('allows SSH before enabling the firewall', () => {
		const commands = firewallCommands('ufw', 2222, true);
		expect(commands.indexOf('sudo ufw allow 2222/tcp')).toBeLessThan(
			commands.indexOf('sudo ufw enable'),
		);
		expect(commands).toContain('sudo ufw allow 443/tcp');
		expect(firewallCommands('firewalld', null, false)).toEqual([
			'sudo firewall-cmd --permanent --add-port=22/tcp',
			'sudo firewall-cmd --reload',
		]);
	});
});
