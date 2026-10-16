type ServerLike = { kind: string };

export function serverGroups<T extends ServerLike>(servers: T[], isCloud: boolean) {
	const remote = servers.filter((server) => server.kind === 'ssh');
	return {
		local: isCloud ? [] : servers.filter((server) => server.kind === 'local'),
		remote,
	};
}

export function canOpenLocalConsole(input: {
	isSysadmin: boolean;
	kind: string;
	state: string;
}): boolean {
	return input.isSysadmin && input.kind === 'local' && input.state === 'ready';
}

export function deploymentTargetNotice(isCloud: boolean, readyServerCount: number): string | null {
	if (isCloud && readyServerCount === 0) {
		return 'Connect and set up a ready remote server before creating a deployment.';
	}
	return null;
}
