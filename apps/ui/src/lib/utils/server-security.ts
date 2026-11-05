import type { ServerSecurity } from '#lib/clients/server-client.js';

export type SecurityStep = 'ssh' | 'fail2ban' | 'firewall' | 'updates';
export type SecurityStepStatus = 'done' | 'todo' | 'unknown';

export function securityStepStatus(
	report: ServerSecurity,
): Record<SecurityStep, SecurityStepStatus> {
	return {
		ssh:
			report.password_login === null
				? 'unknown'
				: !report.password_login && report.root_login !== 'yes'
					? 'done'
					: 'todo',
		fail2ban: report.fail2ban === 'active' ? 'done' : 'todo',
		// A provider firewall is invisible from the host, so this only ever confirms a host one.
		firewall: report.firewall_active ? 'done' : 'todo',
		updates: report.auto_updates === null ? 'unknown' : report.auto_updates ? 'done' : 'todo',
	};
}

/** The SSH allow rule comes before enabling the firewall so the session typing it survives. */
export function firewallCommands(
	firewall: ServerSecurity['firewall'],
	sshPort: number | null,
	publicEdge: boolean,
): string[] {
	const ports = [String(sshPort ?? 22), ...(publicEdge ? ['80', '443'] : [])];
	if (firewall === 'firewalld') {
		return [
			...ports.map((port) => `sudo firewall-cmd --permanent --add-port=${port}/tcp`),
			'sudo firewall-cmd --reload',
		];
	}
	return [
		...(firewall === null ? ['sudo apt-get install -y ufw'] : []),
		...ports.map((port) => `sudo ufw allow ${port}/tcp`),
		'sudo ufw enable',
		'sudo ufw status verbose',
	];
}

export function sshManualCommands(settings: string): string {
	return [
		"sudo tee /etc/ssh/sshd_config.d/00-litepod-hardening.conf >/dev/null <<'EOF'",
		settings.trimEnd(),
		'EOF',
		'sudo sshd -t && (sudo systemctl reload ssh || sudo systemctl reload sshd)',
	].join('\n');
}
